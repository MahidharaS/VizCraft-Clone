//! Report panes: Visualizations (Build / Format / Analytics), Data (fields) and Filters.

use egui::{Color32, RichText, Sense, Ui, vec2};
use vizcraft_core::build::ColType;
use vizcraft_core::doc::*;
use vizcraft_core::format::{self, PropKind};
use vizcraft_core::scalar::Scalar;

use crate::app::VizApp;
use crate::theme::{Tokens, c32};
use crate::widgets;

/// Payload dragged from the Data pane.
#[derive(Clone, Debug)]
pub struct DragField(pub FieldRef);

// ============================================================== Visualizations

pub fn visualizations(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    if widgets::pane_header(ui, t, "Visualizations") {
        app.panes.visualizations = false;
    }
    ui.horizontal(|ui| {
        for (i, label) in ["Build", "Format", "Analytics"].iter().enumerate() {
            let on = app.report.viz_tab == i;
            let r = ui.add(egui::Button::new(RichText::new(*label).color(if on { t.accent } else { t.text }).strong()).frame(false));
            if on {
                ui.painter().line_segment([r.rect.left_bottom(), r.rect.right_bottom()], egui::Stroke::new(2.0, t.accent));
            }
            if r.clicked() {
                app.report.viz_tab = i;
            }
        }
    });
    ui.separator();
    egui::ScrollArea::vertical().id_salt("viz-scroll").auto_shrink([false, false]).show(ui, |ui| match app.report.viz_tab {
        1 => format_tab(app, ui, t),
        2 => analytics_tab(app, ui, t),
        _ => build_tab(app, ui, t),
    });
}

fn selected_visual(app: &VizApp) -> Option<(u64, VisualSpec)> {
    let id = *app.selected.first()?;
    let (_, el) = app.project.element(id)?;
    el.visual().map(|v| (id, v.clone()))
}

fn gallery(app: &mut VizApp, ui: &mut Ui, t: &Tokens, current: Option<VisualKind>) {
    let per_row = ((ui.available_width() / 30.0) as usize).max(4);
    let kinds = VisualKind::ALL;
    for chunk in kinds.chunks(per_row) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for k in chunk {
                let (r, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                if current == Some(*k) {
                    ui.painter().rect_filled(r, 4.0, t.accent_soft);
                    ui.painter().rect_stroke(r, 4.0, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 4.0, t.panel_alt);
                }
                crate::icons::visual(ui.painter(), r.shrink(5.0), *k, t.accent);
                let resp = resp.on_hover_text(k.label());
                if resp.clicked() {
                    match selected_visual(app) {
                        Some((id, _)) => {
                            app.checkpoint("Change visual type");
                            let from = app.project.element(id).and_then(|(_, e)| e.visual().map(|v| v.kind));
                            if let Some(v) = app.project.element_mut(id).and_then(|e| e.visual_mut()) {
                                // Move fields between wells of the new kind where roles differ.
                                remap_wells(v, *k);
                                v.kind = *k;
                            }
                            if let Some(from) = from {
                                crate::ai::track_kind_changed(app, id, from, *k);
                            }
                            app.report.render_cache.clear();
                        }
                        None => super::insert_visual(app, *k),
                    }
                }
            }
        });
    }
}

/// When changing visual type, keep fields in wells with the same role; move the rest sensibly.
pub fn remap_wells(v: &mut VisualSpec, k: VisualKind) {
    let roles: Vec<&str> = k.wells().iter().map(|(r, _)| *r).collect();
    let old = std::mem::take(&mut v.wells);
    let mut cats = Vec::new();
    let mut vals = Vec::new();
    for (role, fields) in old {
        if roles.contains(&role.as_str()) {
            v.wells.insert(role, fields);
        } else {
            for f in fields {
                if f.is_value() { vals.push(f) } else { cats.push(f) }
            }
        }
    }
    let cat_role = ["axis", "rows", "legend", "field", "columns", "location", "details"].into_iter().find(|r| roles.contains(r));
    let val_role = ["values", "y", "columns", "x"].into_iter().find(|r| roles.contains(r));
    if let Some(r) = cat_role {
        v.well_mut(r).extend(cats);
    }
    if let Some(r) = val_role {
        v.well_mut(r).extend(vals);
    }
}

fn build_tab(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let sel = selected_visual(app);
    gallery(app, ui, t, sel.as_ref().map(|s| s.1.kind));
    ui.add_space(6.0);
    let Some((id, v)) = sel else {
        ui.label(RichText::new("Select a visual on the canvas to build it, or click a visual type above to add one.").color(t.muted).size(11.5));
        return;
    };
    ui.label(RichText::new(v.kind.label()).strong());
    for (role, label) in v.kind.wells() {
        well(app, ui, t, id, &v, role, label);
    }
    crate::ai::build_tab_chips(app, ui, t, id);
    if v.kind == VisualKind::Slicer {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Style");
            let mut mode = v.slicer_mode.clone();
            egui::ComboBox::from_id_salt(("slicer-mode", id)).selected_text(&mode).show_ui(ui, |ui| {
                for m in ["List", "Dropdown", "Tile", "Between", "Before", "After", "Relative date"] {
                    ui.selectable_value(&mut mode, m.to_string(), m);
                }
            });
            if mode != v.slicer_mode {
                app.checkpoint("Slicer style");
                if let Some(vv) = app.project.element_mut(id).and_then(|e| e.visual_mut()) {
                    vv.slicer_mode = mode.clone();
                }
                if let Some(e) = app.project.element_mut(id) {
                    let style = match mode.as_str() {
                        "Tile" => "Tile",
                        "Dropdown" => "Dropdown",
                        "Between" => "Between",
                        "Before" => "Less than or equal to",
                        "After" => "Greater than or equal to",
                        "Relative date" => "Relative date",
                        _ => "Vertical list",
                    };
                    e.props.insert("slicer.style".into(), Prop::Choice(style.into()));
                }
                app.report.render_cache.clear();
            }
        });
    }
    if matches!(v.kind, VisualKind::PythonVisual | VisualKind::RVisual) {
        super::script::editor(app, ui, t, id, &v);
    }
    if v.kind == VisualKind::QandA {
        ui.label("Question");
        let mut q = v.question.clone();
        if ui.add(egui::TextEdit::singleline(&mut q).hint_text("e.g. total sales by region")).changed()
            && let Some(vv) = app.project.element_mut(id).and_then(|e| e.visual_mut())
        {
            vv.question = q.clone();
            app.report.render_cache.clear();
        }
        let (_, understood) = vizcraft_core::charts::qna::interpret(&app.project, &app.built, &q);
        if !understood.is_empty() {
            ui.label(RichText::new(format!("Showing: {understood}")).size(10.5).color(t.muted));
        }
        if ui.button("Turn into standard visual").clicked() {
            let (wells, _) = vizcraft_core::charts::qna::interpret(&app.project, &app.built, &q);
            app.checkpoint("Q&A to visual");
            if let Some(vv) = app.project.element_mut(id).and_then(|e| e.visual_mut()) {
                vv.kind = VisualKind::ClusteredColumn;
                vv.wells = wells;
            }
            app.report.render_cache.clear();
        }
    }
    ui.add_space(8.0);
    // Drill-through + tooltip page.
    widgets::section(ui, "drill-sec", "Drill through", false, |ui| {
        ui.label(RichText::new("Right-click a data point → Drill through, or set a page's drill-through fields in its page settings.").size(10.5).color(t.muted));
        let dt_pages: Vec<(u64, String)> = app.project.report.pages.iter().filter(|p| !p.drillthrough.is_empty()).map(|p| (p.id, p.name.clone())).collect();
        for (pid, name) in dt_pages {
            if ui.button(format!("Drill through to {name}")).clicked() {
                super::drillthrough_from_selection(app, id, pid);
            }
        }
    });
}

fn well(app: &mut VizApp, ui: &mut Ui, t: &Tokens, id: u64, v: &VisualSpec, role: &str, label: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(label).size(11.5).color(t.muted));
    let fields = v.well(role).to_vec();
    let frame = egui::Frame::new().fill(t.panel_alt).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(4.0).inner_margin(egui::Margin::same(4));
    let (resp, payload) = ui.dnd_drop_zone::<DragField, ()>(frame, |ui| {
        ui.set_min_width(ui.available_width());
        if fields.is_empty() {
            ui.label(RichText::new("Add data fields here").size(11.0).color(t.muted));
        }
        for (i, f) in fields.iter().enumerate() {
            let chip = widgets::chip(ui, t, &f.label(), f.is_value(), false);
            chip_menu(app, &chip, id, role, i, f);
            if chip.drag_started() {
                app.report.dragging_well = Some((id, role.to_string(), i));
            }
        }
    });
    let _ = resp;
    if let Some(p) = payload {
        add_field(app, id, role, p.0.clone());
    }
}

fn chip_menu(app: &mut VizApp, chip: &egui::Response, id: u64, role: &str, i: usize, f: &FieldUse) {
    chip.context_menu(|ui| chip_menu_body(app, ui, id, role, i, f));
    if chip.clicked() {
        app.report.chip_menu = Some((id, role.to_string(), i));
    }
    if app.report.chip_menu.as_ref() == Some(&(id, role.to_string(), i)) {
        egui::Popup::from_response(chip).open_memory(None).show(|ui| {
            ui.set_min_width(200.0);
            chip_menu_body(app, ui, id, role, i, f);
        });
        if chip.ctx.input(|inp| inp.pointer.any_click()) && !chip.clicked() && !chip.ctx.is_pointer_over_egui() {
            app.report.chip_menu = None;
        }
    }
}

fn chip_menu_body(app: &mut VizApp, ui: &mut Ui, id: u64, role: &str, i: usize, f: &FieldUse) {
    let mut fu = f.clone();
    let mut changed = false;
    let mut remove = false;
    let mut mv: i32 = 0;
    if !matches!(fu.field, FieldRef::Measure { .. }) {
        ui.label(RichText::new("Summarize").strong());
        for a in Agg::ALL {
            if ui.radio(fu.agg == a, a.label()).clicked() {
                fu.agg = a;
                changed = true;
            }
        }
        ui.separator();
    }
    ui.menu_button("Show value as", |ui| {
        for (s, label) in [
            (ShowAs::Value, "No calculation"),
            (ShowAs::PercentOfGrandTotal, "Percent of grand total"),
            (ShowAs::PercentOfColumnTotal, "Percent of column total"),
            (ShowAs::RunningTotal, "Running total"),
            (ShowAs::DifferenceFromPrevious, "Difference from previous"),
            (ShowAs::PercentDifferenceFromPrevious, "% difference from previous"),
            (ShowAs::Rank, "Rank"),
        ] {
            if ui.radio(fu.show_as == s, label).clicked() {
                fu.show_as = s;
                changed = true;
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("Rename");
        let mut alias = fu.alias.clone().unwrap_or_default();
        if ui.text_edit_singleline(&mut alias).changed() {
            fu.alias = if alias.is_empty() { None } else { Some(alias) };
            changed = true;
        }
    });
    if !fu.is_value() {
        ui.horizontal(|ui| {
            let mut on = fu.bin.is_some();
            if ui.checkbox(&mut on, "Bins").changed() {
                fu.bin = if on { Some(10.0) } else { None };
                changed = true;
            }
            if let Some(b) = fu.bin.as_mut() {
                changed |= ui.add(egui::DragValue::new(b).speed(1.0).range(0.0001..=1e9)).changed();
            }
        });
        if ui.checkbox(&mut fu.show_empty, "Show items with no data").changed() {
            changed = true;
        }
    }
    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("▲").on_hover_text("Move up").clicked() {
            mv = -1;
        }
        if ui.button("▼").on_hover_text("Move down").clicked() {
            mv = 1;
        }
        if ui.button("Remove field").clicked() {
            remove = true;
        }
    });
    if changed || remove || mv != 0 {
        app.checkpoint("Edit field");
        if let Some(v) = app.project.element_mut(id).and_then(|e| e.visual_mut()) {
            let w = v.well_mut(role);
            if remove {
                if i < w.len() {
                    w.remove(i);
                }
                ui.close();
            } else if mv != 0 {
                let j = i as i32 + mv;
                if j >= 0 && (j as usize) < w.len() {
                    w.swap(i, j as usize);
                }
            } else if let Some(slot) = w.get_mut(i) {
                *slot = fu;
            }
        }
        app.report.render_cache.clear();
    }
}

/// Adds a field to a well, choosing the aggregation for value wells.
pub fn add_field(app: &mut VizApp, id: u64, role: &str, f: FieldRef) {
    let value_role = matches!(role, "values" | "secondary" | "x" | "y" | "size" | "target" | "min" | "max" | "tooltips" | "lat" | "lon" | "start");
    let mut fu = FieldUse::new(f.clone());
    if value_role
        && let FieldRef::Column { table, column } = &f
    {
        fu.agg = vizcraft_core::state::default_agg(&app.project, &app.built, table, column);
    }
    app.checkpoint("Add field");
    if let Some(v) = app.project.element_mut(id).and_then(|e| e.visual_mut()) {
        let w = v.well_mut(role);
        if !w.iter().any(|x| x.field == fu.field && x.agg == fu.agg) {
            if matches!(role, "field") {
                w.clear();
            }
            w.push(fu);
        }
    }
    app.report.render_cache.clear();
    crate::ai::track_field_added(app, id, role, &f);
}

/// Clicking a field in the Data pane: adds it to the selected visual's best well (or creates a
/// visual like Power BI does).
pub fn click_field(app: &mut VizApp, f: FieldRef) {
    let is_value = f.is_measure() || matches!(&f, FieldRef::Column { table, column } if app.built.col_type(table, column).is_numeric() && !column.to_lowercase().contains("id") && !column.to_lowercase().contains("year"));
    let sel = selected_visual(app);
    let (id, v) = match sel {
        Some(s) => s,
        None => {
            let kind = if is_value { VisualKind::Card } else { VisualKind::Table };
            super::insert_visual(app, kind);
            match selected_visual(app) {
                Some(s) => s,
                None => return,
            }
        }
    };
    // Already there → remove (toggle like the checkbox).
    for (role, fields) in &v.wells {
        if let Some(i) = fields.iter().position(|x| x.field == f) {
            app.checkpoint("Remove field");
            if let Some(vv) = app.project.element_mut(id).and_then(|e| e.visual_mut()) {
                vv.well_mut(role).remove(i);
            }
            app.report.render_cache.clear();
            return;
        }
    }
    let roles: Vec<&str> = v.kind.wells().iter().map(|(r, _)| *r).collect();
    let role = if v.kind == VisualKind::Table {
        "columns"
    } else if is_value {
        ["values", "y", "x", "size", "field"].into_iter().find(|r| roles.contains(r) && (*r != "x" || v.well("x").is_empty()) && (*r != "y" || v.well("y").is_empty())).unwrap_or("values")
    } else {
        ["axis", "rows", "legend", "field", "location", "details"].into_iter().find(|r| roles.contains(r) && (v.well(r).is_empty() || matches!(*r, "axis" | "rows"))).unwrap_or("axis")
    };
    if v.kind == VisualKind::Card && !is_value {
        // A card with a category becomes a table.
        app.checkpoint("Table");
        if let Some(vv) = app.project.element_mut(id).and_then(|e| e.visual_mut()) {
            vv.kind = VisualKind::Table;
            let vals = vv.well("values").to_vec();
            vv.wells.clear();
            vv.set("columns", vals);
        }
        add_field(app, id, "columns", f);
        return;
    }
    let mut fu_f = f;
    if !is_value && v.kind == VisualKind::Card {
        fu_f = fu_f.clone();
    }
    add_field(app, id, role, fu_f);
}

// ============================================================== Format

fn format_tab(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let Some(id) = app.selected.first().copied() else {
        super::pages::page_format(app, ui, t);
        return;
    };
    let Some((page_id, el)) = app.project.element(id).map(|(p, e)| (p, e.clone())) else { return };
    ui.add(egui::TextEdit::singleline(&mut app.report.format_search).hint_text("Search format options"));
    let search = app.report.format_search.to_lowercase();
    // General geometry.
    widgets::section(ui, "fmt-general", "Properties", true, |ui| {
        let mut e = el.clone();
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Name");
            changed |= ui.text_edit_singleline(&mut e.name).changed();
        });
        egui::Grid::new("geom").num_columns(4).show(ui, |ui| {
            ui.label("X");
            changed |= ui.add(egui::DragValue::new(&mut e.x).speed(1.0)).changed();
            ui.label("Y");
            changed |= ui.add(egui::DragValue::new(&mut e.y).speed(1.0)).changed();
            ui.end_row();
            ui.label("W");
            changed |= ui.add(egui::DragValue::new(&mut e.w).speed(1.0).range(4.0..=10000.0)).changed();
            ui.label("H");
            changed |= ui.add(egui::DragValue::new(&mut e.h).speed(1.0).range(4.0..=10000.0)).changed();
            ui.end_row();
            ui.label("Rotate");
            changed |= ui.add(egui::DragValue::new(&mut e.rotation).speed(1.0).suffix("°")).changed();
            ui.label("Padding");
            changed |= ui.add(egui::DragValue::new(&mut e.style.padding).speed(0.5).range(0.0..=200.0)).changed();
            ui.end_row();
        });
        ui.horizontal(|ui| {
            ui.label("Opacity");
            changed |= ui.add(egui::Slider::new(&mut e.style.opacity, 0.0..=1.0)).changed();
        });
        ui.horizontal(|ui| {
            changed |= ui.checkbox(&mut e.locked, "Lock").changed();
            changed |= ui.checkbox(&mut e.hidden, "Hidden").changed();
            changed |= ui.checkbox(&mut e.floating, "Floating").on_hover_text("Dashboard layout: floating vs tiled").changed();
        });
        if e.visual().is_some_and(|v| v.kind == VisualKind::Slicer) {
            ui.horizontal(|ui| {
                ui.label("Sync group");
                let mut g = e.sync_group.clone().unwrap_or_default();
                if ui.text_edit_singleline(&mut g).on_hover_text("Slicers with the same sync group stay in step across pages").changed() {
                    e.sync_group = if g.is_empty() { None } else { Some(g) };
                    changed = true;
                }
            });
        }
        if e.visual().is_some() {
            let tips: Vec<(u64, String)> = app.project.report.pages.iter().filter(|p| p.kind == PageKind::Tooltip).map(|p| (p.id, p.name.clone())).collect();
            ui.horizontal(|ui| {
                ui.label("Tooltip page");
                let cur = e.tooltip_page.and_then(|id| tips.iter().find(|(i, _)| *i == id)).map(|x| x.1.clone()).unwrap_or_else(|| "Default".into());
                egui::ComboBox::from_id_salt(("tip-page", id)).selected_text(cur).show_ui(ui, |ui| {
                    if ui.selectable_label(e.tooltip_page.is_none(), "Default").clicked() {
                        e.tooltip_page = None;
                        changed = true;
                    }
                    for (pid, name) in &tips {
                        if ui.selectable_label(e.tooltip_page == Some(*pid), name).clicked() {
                            e.tooltip_page = Some(*pid);
                            changed = true;
                        }
                    }
                });
            });
        }
        if changed {
            app.checkpoint("Format");
            if let Some(x) = app.project.element_mut(id) {
                *x = e;
            }
            app.report.render_cache.clear();
        }
    });
    match &el.kind {
        ElementKind::Shape(_) => super::design::shape_props(app, ui, t, id),
        ElementKind::Text(_) => super::design::text_props(app, ui, t, id),
        ElementKind::Button(_) => super::design::button_props(app, ui, t, id),
        ElementKind::Image(_) => super::design::image_props(app, ui, t, id),
        _ => {}
    }
    super::design::box_style(app, ui, t, id);
    let key = format::schema_key(&el);
    for card in format::cards(&key) {
        let matches: Vec<&format::PropDef> = card.props.iter().filter(|p| search.is_empty() || p.label.to_lowercase().contains(&search) || card.name.to_lowercase().contains(&search)).collect();
        if matches.is_empty() && card.name != "Data colors" {
            continue;
        }
        widgets::section(ui, &format!("card-{}-{}", key, card.name), card.name, !search.is_empty(), |ui| {
            for pd in matches {
                prop_row(app, ui, t, id, pd);
            }
            if card.name == "Data colors" {
                data_colors_extra(app, ui, t, id, page_id);
            }
        });
    }
    if matches!(el.visual().map(|v| v.kind), Some(VisualKind::Table | VisualKind::Matrix | VisualKind::HighlightTable)) {
        widgets::section(ui, "cf", "Cell elements (conditional formatting)", false, |ui| cond_format(app, ui, t, id, page_id));
    }
}

/// One property row: label, editor, fx button.
pub fn prop_row(app: &mut VizApp, ui: &mut Ui, t: &Tokens, id: u64, pd: &format::PropDef) {
    let Some((_, el)) = app.project.element(id) else { return };
    let cur = el.props.get(pd.key).cloned();
    let value = cur.clone().unwrap_or_else(|| pd.default.clone());
    ui.horizontal(|ui| {
        ui.add_sized([110.0, 18.0], egui::Label::new(RichText::new(pd.label).size(11.5)).truncate());
        let mut newv: Option<Prop> = None;
        if let Prop::Fx(fx) = &value {
            let summary = super::fx_dialog::summary(fx);
            if ui.add(egui::Button::new(RichText::new(format!("ƒx {summary}")).size(11.0).color(t.accent))).on_hover_text("Bound to data — click to edit").clicked() {
                super::fx_dialog::open(app, id, pd.key, &pd.kind, Some(fx.clone()));
            }
            if ui.small_button("✕").on_hover_text("Remove the binding").clicked() {
                app.checkpoint("Clear fx");
                if let Some(e) = app.project.element_mut(id) {
                    e.props.remove(pd.key);
                }
                app.report.render_cache.clear();
            }
            return;
        }
        match &pd.kind {
            PropKind::Bool => {
                let mut b = value.as_bool().unwrap_or(false);
                if ui.checkbox(&mut b, "").changed() {
                    newv = Some(Prop::Bool(b));
                }
            }
            PropKind::Num { min, max, step } => {
                let mut x = value.as_num().unwrap_or(0.0);
                if ui.add(egui::DragValue::new(&mut x).range(*min..=*max).speed(*step)).changed() {
                    newv = Some(Prop::Num(x));
                }
            }
            PropKind::Text => {
                let mut s = value.as_text();
                if ui.add(egui::TextEdit::singleline(&mut s).desired_width(110.0)).changed() {
                    newv = Some(Prop::Text(s));
                }
            }
            PropKind::Template => {
                let mut s = value.as_text();
                if ui.add(egui::TextEdit::singleline(&mut s).desired_width(110.0).hint_text("{[Measure]}")).on_hover_text("Templates: {[Measure]}, {[Measure]:$#,0}, {SUM(T[C])}, {Selected:Field}, {@Param}, {Page}").changed() {
                    newv = Some(if s.contains('{') { Prop::Fx(Fx::Template { text: s }) } else { Prop::Text(s) });
                }
            }
            PropKind::Color => {
                let c = value.as_color();
                let mut cc = c.unwrap_or(vizcraft_core::color::Color::rgba(0, 0, 0, 0));
                let mut c32v = c32(cc);
                if c.is_none() {
                    let r = ui.add(egui::Button::new("Auto").small());
                    if r.clicked() {
                        newv = Some(Prop::Color(vizcraft_core::color::Color::rgb(0x2B, 0x6C, 0xE0)));
                    }
                } else if egui::color_picker::color_edit_button_srgba(ui, &mut c32v, egui::color_picker::Alpha::OnlyBlend).changed() {
                    cc = crate::theme::from32(c32v);
                    newv = Some(Prop::Color(cc));
                }
                if c.is_some() && ui.small_button("↺").on_hover_text("Reset to default").clicked() {
                    newv = Some(pd.default.clone());
                }
            }
            PropKind::Choice(opts) => {
                let mut s = value.as_text();
                egui::ComboBox::from_id_salt(("choice", id, pd.key)).width(110.0).selected_text(&s).show_ui(ui, |ui| {
                    for o in opts.iter() {
                        if ui.selectable_value(&mut s, o.to_string(), *o).clicked() {
                            newv = Some(Prop::Choice(s.clone()));
                        }
                    }
                });
            }
        }
        if ui.add(egui::Button::new(RichText::new("ƒx").size(11.0).color(t.muted)).frame(false)).on_hover_text("Conditional / dynamic value (fx)").clicked() {
            super::fx_dialog::open(app, id, pd.key, &pd.kind, None);
        }
        if let Some(v) = newv {
            app.checkpoint("Format");
            crate::ai::track_format(app, id, pd.key, &v);
            if let Some(e) = app.project.element_mut(id) {
                e.props.insert(pd.key.to_string(), v);
            }
            app.report.render_cache.clear();
        }
    });
}

fn data_colors_extra(app: &mut VizApp, ui: &mut Ui, t: &Tokens, id: u64, page_id: u64) {
    let fx_on = app.project.element(id).is_some_and(|(_, e)| e.props.contains_key("dataColors.fx"));
    ui.horizontal(|ui| {
        ui.label(RichText::new("Colors by data (fx)").size(11.5));
        if ui.button(if fx_on { "Edit fx" } else { "ƒx" }).on_hover_text("Colour each data point by a rule, gradient or field value").clicked() {
            let cur = app.project.element(id).and_then(|(_, e)| e.props.get("dataColors.fx")).and_then(|p| if let Prop::Fx(f) = p { Some(f.clone()) } else { None });
            super::fx_dialog::open(app, id, "dataColors.fx", &PropKind::Color, cur);
        }
        if fx_on && ui.small_button("✕").clicked() {
            app.checkpoint("Clear data colors fx");
            if let Some(e) = app.project.element_mut(id) {
                e.props.remove("dataColors.fx");
            }
            app.report.render_cache.clear();
        }
    });
    // Per-point colours.
    let data = app.report.render_cache.get(&(id, page_id)).and_then(|(_, r)| r.data.clone());
    let Some(d) = data else { return };
    let show_all = app.project.element(id).is_some_and(|(_, e)| e.props.get("dataColors.showAll").and_then(|p| p.as_bool()).unwrap_or(false));
    let multi_series = d.values().len() > 1 || d.first("legend").is_some();
    if !show_all && !multi_series {
        return;
    }
    let mut items: Vec<(String, String)> = Vec::new();
    if let Some(li) = d.first("legend").or_else(|| if show_all { d.categories().first().copied() } else { None }) {
        for v in d.distinct(li).into_iter().take(40) {
            items.push((v.key(), v.text()));
        }
    } else {
        for (i, vi) in d.values().into_iter().enumerate() {
            items.push((format!("series:{i}"), d.cols.get(vi).map(|c| c.label.clone()).unwrap_or_default()));
        }
    }
    for (i, (k, label)) in items.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.add_sized([110.0, 18.0], egui::Label::new(RichText::new(label).size(11.0)).truncate());
            let key = format!("dataColors.point.{k}");
            let cur = app.project.element(id).and_then(|(_, e)| e.props.get(&key)).and_then(|p| p.as_color()).unwrap_or(app.project.report.theme.series_color(i));
            let mut c = c32(cur);
            if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::OnlyBlend).changed() {
                app.checkpoint("Data color");
                if let Some(e) = app.project.element_mut(id) {
                    e.props.insert(key, Prop::Color(crate::theme::from32(c)));
                }
                app.report.render_cache.clear();
            }
        });
    }
    let _ = t;
}

fn cond_format(app: &mut VizApp, ui: &mut Ui, t: &Tokens, id: u64, page_id: u64) {
    let data = app.report.render_cache.get(&(id, page_id)).and_then(|(_, r)| r.data.clone());
    let Some(d) = data else { return };
    ui.label(RichText::new("Pick a column, then bind its background, font colour, data bars or icons to rules, gradients or a field.").size(10.5).color(t.muted));
    for c in &d.cols {
        ui.horizontal(|ui| {
            ui.add_sized([100.0, 18.0], egui::Label::new(RichText::new(&c.label).size(11.0)).truncate());
            for (aspect, label) in [("background", "Bg"), ("font", "Font"), ("dataBar", "Bars"), ("icon", "Icons")] {
                let key = format!("cf.{}.{aspect}", c.label);
                let on = app.project.element(id).is_some_and(|(_, e)| e.props.contains_key(&key));
                let r = ui.add(egui::Button::new(RichText::new(label).size(10.5).color(if on { Color32::WHITE } else { t.text })).fill(if on { t.accent } else { t.panel_alt }));
                if r.clicked() {
                    let cur = app.project.element(id).and_then(|(_, e)| e.props.get(&key)).and_then(|p| if let Prop::Fx(f) = p { Some(f.clone()) } else { None });
                    let default = cur.or_else(|| {
                        Some(Fx::Gradient { dax: String::new(), min: None, max: None, min_color: vizcraft_core::color::Color::rgb(0xFE, 0xE2, 0xE2), mid_color: None, max_color: vizcraft_core::color::Color::rgb(0xBB, 0xF7, 0xD0) })
                    });
                    super::fx_dialog::open_owned(app, id, key.clone(), PropKind::Color, default);
                }
                if on && r.secondary_clicked() {
                    app.checkpoint("Remove conditional formatting");
                    if let Some(e) = app.project.element_mut(id) {
                        e.props.remove(&key);
                    }
                    app.report.render_cache.clear();
                }
            }
        });
    }
}

// ============================================================== Analytics

fn analytics_tab(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let Some((id, v)) = selected_visual(app) else {
        ui.label(RichText::new("Select a chart to add reference lines, trend lines, forecasts and clusters.").color(t.muted).size(11.5));
        return;
    };
    ui.label(RichText::new("Add").strong());
    ui.horizontal_wrapped(|ui| {
        for kind in ["Constant", "Min", "Max", "Average", "Median", "Percentile", "Trend", "Forecast", "Error bars", "Reference band", "Cluster", "Anomalies"] {
            if ui.button(kind).clicked() {
                app.checkpoint("Add analytics");
                if let Some(vv) = app.project.element_mut(id).and_then(|e| e.visual_mut()) {
                    vv.analytics.push(AnalyticsLine { kind: kind.into(), name: format!("{kind} line"), ..AnalyticsLine::default() });
                }
                app.report.render_cache.clear();
            }
        }
    });
    ui.separator();
    let mut lines = v.analytics.clone();
    let mut changed = false;
    let mut remove = None;
    for (i, l) in lines.iter_mut().enumerate() {
        egui::Frame::new().fill(t.panel_alt).corner_radius(4.0).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&l.kind).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").clicked() {
                        remove = Some(i);
                    }
                });
            });
            egui::Grid::new(("an", i)).num_columns(2).show(ui, |ui| {
                ui.label("Name");
                changed |= ui.text_edit_singleline(&mut l.name).changed();
                ui.end_row();
                ui.label("Series");
                changed |= ui.text_edit_singleline(&mut l.series).changed();
                ui.end_row();
                match l.kind.as_str() {
                    "Constant" | "Reference band" => {
                        ui.label("Value");
                        let mut x = l.value.as_num().unwrap_or(0.0);
                        if ui.add(egui::DragValue::new(&mut x).speed(1.0)).changed() {
                            l.value = Prop::Num(x);
                            changed = true;
                        }
                        ui.end_row();
                        ui.label("Value (DAX)");
                        let mut dax = match &l.value {
                            Prop::Fx(Fx::Expression { dax }) => dax.clone(),
                            _ => String::new(),
                        };
                        if ui.text_edit_singleline(&mut dax).on_hover_text("Optional: a DAX expression or @Parameter").changed() {
                            l.value = if dax.is_empty() { Prop::Num(0.0) } else { Prop::Fx(Fx::Expression { dax }) };
                            changed = true;
                        }
                        ui.end_row();
                    }
                    "Percentile" => {
                        ui.label("Percentile");
                        changed |= ui.add(egui::Slider::new(&mut l.percentile, 0.0..=100.0)).changed();
                        ui.end_row();
                    }
                    "Forecast" => {
                        ui.label("Length");
                        changed |= ui.add(egui::DragValue::new(&mut l.forecast_length).range(1..=120)).changed();
                        ui.end_row();
                        ui.label("Seasonality");
                        changed |= ui.add(egui::DragValue::new(&mut l.seasonality).range(0..=366)).on_hover_text("0 = detect automatically").changed();
                        ui.end_row();
                        ui.label("Confidence");
                        changed |= ui.add(egui::Slider::new(&mut l.confidence, 0.5..=0.99)).changed();
                        ui.end_row();
                        ui.label("Ignore last");
                        changed |= ui.add(egui::DragValue::new(&mut l.ignore_last).range(0..=60)).changed();
                        ui.end_row();
                    }
                    "Cluster" => {
                        ui.label("Clusters");
                        changed |= ui.add(egui::DragValue::new(&mut l.clusters).range(2..=12)).changed();
                        ui.end_row();
                    }
                    _ => {}
                }
                ui.label("Color");
                changed |= widgets::color_edit(ui, &mut l.color).changed();
                ui.end_row();
                ui.label("Width");
                changed |= ui.add(egui::DragValue::new(&mut l.width).range(0.5..=8.0).speed(0.1)).changed();
                ui.end_row();
                ui.label("Style");
                egui::ComboBox::from_id_salt(("dash", i)).selected_text(&l.dash).show_ui(ui, |ui| {
                    for s in ["solid", "dash", "dot"] {
                        changed |= ui.selectable_value(&mut l.dash, s.to_string(), s).changed();
                    }
                });
                ui.end_row();
                ui.label("Data label");
                changed |= ui.checkbox(&mut l.show_label, "").changed();
                ui.end_row();
            });
        });
        ui.add_space(4.0);
    }
    if let Some(i) = remove {
        lines.remove(i);
        changed = true;
    }
    if changed {
        app.checkpoint("Analytics");
        if let Some(vv) = app.project.element_mut(id).and_then(|e| e.visual_mut()) {
            vv.analytics = lines;
        }
        app.report.render_cache.clear();
    }
}

// ============================================================== Data pane

fn type_glyph(ty: ColType) -> &'static str {
    match ty {
        ColType::Number | ColType::Integer => "∑",
        ColType::Date | ColType::DateTime => "◷",
        ColType::Bool => "◑",
        _ => "",
    }
}

pub fn data_pane(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    if widgets::pane_header(ui, t, "Data") {
        app.panes.data = false;
    }
    ui.add(egui::TextEdit::singleline(&mut app.report.field_search).hint_text("Search fields"));
    ui.add_space(4.0);
    let q = app.report.field_search.to_lowercase();
    let used: Vec<FieldRef> = selected_visual(app).map(|(_, v)| v.all_fields().map(|f| f.field.clone()).collect()).unwrap_or_default();
    let tables: Vec<ModelTable> = app.project.model.tables.iter().filter(|t| !t.hidden).cloned().collect();
    egui::ScrollArea::vertical().id_salt("data-scroll").auto_shrink([false, false]).show(ui, |ui| {
        for tbl in &tables {
            let cols = vizcraft_core::build::visible_columns(&app.project, &app.built, &tbl.name);
            let measures: Vec<Measure> = app.project.model.measures.iter().filter(|m| m.table == tbl.name && !m.hidden).cloned().collect();
            let is_param = app.project.parameter(&tbl.name).is_some();
            let header = format!("{}  {}", if is_param { "◇" } else { "▦" }, tbl.name);
            let any_match = q.is_empty() || tbl.name.to_lowercase().contains(&q) || cols.iter().any(|(c, _)| c.to_lowercase().contains(&q)) || measures.iter().any(|m| m.name.to_lowercase().contains(&q));
            if !any_match {
                continue;
            }
            let resp = egui::CollapsingHeader::new(RichText::new(header).strong()).id_salt(("tbl", &tbl.name)).default_open(!q.is_empty() || tables.len() <= 4).show(ui, |ui| {
                for m in &measures {
                    if !q.is_empty() && !m.name.to_lowercase().contains(&q) {
                        continue;
                    }
                    let f = FieldRef::measure(&tbl.name, &m.name);
                    field_item(app, ui, t, f, &m.name, "▣", used.iter().any(|u| *u == FieldRef::measure(&tbl.name, &m.name)), true);
                }
                for (c, ty) in &cols {
                    if !q.is_empty() && !c.to_lowercase().contains(&q) {
                        continue;
                    }
                    let f = FieldRef::col(&tbl.name, c);
                    if ty.is_temporal() && app.project.model.auto_date_time {
                        egui::CollapsingHeader::new(format!("◷  {c}")).id_salt(("dh", &tbl.name, c)).show(ui, |ui| {
                            field_item(app, ui, t, f.clone(), &format!("{c} (date)"), "◷", used.contains(&f), false);
                            for lvl in vizcraft_core::build::DATE_LEVELS {
                                let lf = FieldRef::Level { table: tbl.name.clone(), hierarchy: c.clone(), level: lvl.into() };
                                let u = used.contains(&lf);
                                field_item(app, ui, t, lf, lvl, "·", u, false);
                            }
                        });
                    } else {
                        field_item(app, ui, t, f.clone(), c, type_glyph(*ty), used.contains(&f), false);
                    }
                }
                for h in &tbl.hierarchies {
                    egui::CollapsingHeader::new(format!("⌗  {}", h.name)).id_salt(("h", &tbl.name, &h.name)).show(ui, |ui| {
                        for l in &h.levels {
                            let lf = FieldRef::Level { table: tbl.name.clone(), hierarchy: h.name.clone(), level: l.clone() };
                            let u = used.contains(&lf);
                            field_item(app, ui, t, lf, l, "·", u, false);
                        }
                    });
                }
            });
            resp.header_response.context_menu(|ui| {
                if ui.button("New measure").clicked() {
                    crate::model::new_measure(app, Some(&tbl.name));
                    ui.close();
                }
                if ui.button("New column").clicked() {
                    crate::model::new_column(app, Some(&tbl.name));
                    ui.close();
                }
                if ui.button("Edit query").clicked() {
                    crate::pq::open_editor(app, Some(&tbl.name));
                    ui.close();
                }
                if ui.button("Edit as data sheet").clicked() {
                    crate::sheet::sheet_from_query(app, &tbl.name);
                    ui.close();
                }
            });
        }
        // Field parameters and value parameters.
        let params: Vec<Parameter> = app.project.model.parameters.iter().filter(|p| matches!(p.kind, ParameterKind::Fields { .. } | ParameterKind::Value { .. })).cloned().collect();
        if !params.is_empty() {
            ui.separator();
            ui.label(RichText::new("Parameters").strong());
            for prm in params {
                match &prm.kind {
                    ParameterKind::Fields { .. } => {
                        let f = FieldRef::FieldParam { name: prm.name.clone() };
                        let u = used.contains(&f);
                        field_item(app, ui, t, f, &format!("{} (field parameter)", prm.name), "◇", u, false);
                    }
                    ParameterKind::Value { list, min, max, step, .. } => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("@{}", prm.name)).color(t.accent));
                            let mut cur = prm.current.clone();
                            let mut changed = false;
                            if !list.is_empty() {
                                egui::ComboBox::from_id_salt(("vp", &prm.name)).selected_text(cur.text()).show_ui(ui, |ui| {
                                    for v in list {
                                        if ui.selectable_label(v.key() == cur.key(), v.text()).clicked() {
                                            cur = v.clone();
                                            changed = true;
                                        }
                                    }
                                });
                            } else if let (Some(a), Some(b)) = (min, max) {
                                let mut x = cur.as_f64().unwrap_or(*a);
                                if ui.add(egui::Slider::new(&mut x, *a..=*b).step_by(step.unwrap_or(0.0))).changed() {
                                    cur = Scalar::Num(x);
                                    changed = true;
                                }
                            } else {
                                let mut s = cur.text();
                                if ui.text_edit_singleline(&mut s).changed() {
                                    cur = vizcraft_core::sheet::parse_input(&s);
                                    changed = true;
                                }
                            }
                            if changed {
                                super::actions::set_parameter(app, &prm.name, cur);
                            }
                        });
                    }
                    _ => {}
                }
            }
        }
        if tables.is_empty() {
            ui.add_space(20.0);
            ui.label(RichText::new("No data yet").strong());
            ui.label(RichText::new("Home → Get data, or open the Retail sample (File).").color(t.muted));
            if ui.button("Get data").clicked() {
                crate::pq::get_data(app, "more");
            }
            if ui.button("Open Retail sample").clicked() {
                app.load_project(vizcraft_core::sample::sales(), None);
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn field_item(app: &mut VizApp, ui: &mut Ui, t: &Tokens, f: FieldRef, label: &str, glyph: &str, used: bool, measure: bool) {
    let id = ui.scope_id().with(("field", format!("{f:?}")));
    let resp = ui
        .dnd_drag_source(id, DragField(f.clone()), |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                ui.painter().rect(r, 2.0, if used { t.accent } else { Color32::TRANSPARENT }, egui::Stroke::new(1.0, if used { t.accent } else { t.muted }), egui::StrokeKind::Inside);
                if used {
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "✓", egui::FontId::proportional(10.0), Color32::WHITE);
                }
                ui.label(RichText::new(glyph).color(if measure { t.good } else { t.muted }).size(11.0));
                ui.label(RichText::new(label).size(12.0));
            });
        })
        .response;
    let resp = resp.interact(Sense::click());
    if resp.clicked() {
        click_field(app, f.clone());
    }
    resp.context_menu(|ui| {
        if ui.button("Add to filters on this page").clicked() {
            super::filters::add_filter(app, "page", f.clone());
            ui.close();
        }
        if ui.button("Add to filters on all pages").clicked() {
            super::filters::add_filter(app, "report", f.clone());
            ui.close();
        }
        if ui.button("Add as slicer").clicked() {
            super::insert_visual(app, VisualKind::Slicer);
            if let Some(id) = app.selected.first().copied() {
                add_field(app, id, "field", f.clone());
            }
            ui.close();
        }
        if ui.button("Add to drill-through (this page)").clicked() {
            app.checkpoint("Drill-through field");
            if let Some(pg) = app.current_page_mut() {
                pg.drillthrough.push(f.clone());
                if pg.kind == PageKind::Report {
                    pg.kind = PageKind::Drillthrough;
                }
            }
            ui.close();
        }
        if measure {
            if ui.button("Edit measure").clicked() {
                if let FieldRef::Measure { table, .. } = &f {
                    crate::model::new_measure(app, Some(table));
                }
                ui.close();
            }
        } else if ui.button("New measure here").clicked() {
            crate::model::new_measure(app, Some(f.table()));
            ui.close();
        }
    });
}
