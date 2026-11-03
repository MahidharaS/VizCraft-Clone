//! Tableau-style worksheets: Columns / Rows shelves, the Marks card (mark type, Color, Size,
//! Label, Detail, Tooltip), a Filters shelf and Show Me. The shelves drive the visual's wells.

use egui::{RichText, Ui};
use vizcraft_core::doc::*;

use super::panes::DragField;
use crate::app::VizApp;
use crate::theme::Tokens;
use crate::widgets;

pub const MARKS: &[&str] = &["Automatic", "Bar", "Line", "Area", "Square", "Circle", "Text", "Pie", "Gantt Bar"];

/// Recomputes a worksheet visual's kind and wells from its shelves and marks.
pub fn apply_shelves(app: &VizApp, v: &mut VisualSpec) {
    let cols = v.well("ws.columns").to_vec();
    let rows = v.well("ws.rows").to_vec();
    let is_val = |f: &FieldUse| f.is_value();
    let col_meas: Vec<FieldUse> = cols.iter().filter(|f| is_val(f)).cloned().collect();
    let row_meas: Vec<FieldUse> = rows.iter().filter(|f| is_val(f)).cloned().collect();
    let col_dims: Vec<FieldUse> = cols.iter().filter(|f| !is_val(f)).cloned().collect();
    let row_dims: Vec<FieldUse> = rows.iter().filter(|f| !is_val(f)).cloned().collect();
    let date_dim = col_dims.iter().chain(row_dims.iter()).any(|f| match &f.field {
        FieldRef::Level { .. } => true,
        FieldRef::Column { table, column } => app.built.col_type(table, column).is_temporal(),
        _ => false,
    });
    let mark = if v.marks.mark.is_empty() { "Automatic".to_string() } else { v.marks.mark.clone() };
    let horizontal = !col_meas.is_empty() && row_meas.is_empty();
    let kind = match mark.as_str() {
        "Line" => VisualKind::Line,
        "Area" => VisualKind::Area,
        "Square" => VisualKind::HeatMap,
        "Circle" => {
            if !col_meas.is_empty() && !row_meas.is_empty() {
                VisualKind::Scatter
            } else {
                VisualKind::PackedBubbles
            }
        }
        "Text" => VisualKind::Table,
        "Pie" => VisualKind::Pie,
        "Gantt Bar" => VisualKind::Gantt,
        "Bar" => {
            if horizontal {
                VisualKind::StackedBar
            } else {
                VisualKind::StackedColumn
            }
        }
        _ => {
            if col_meas.is_empty() && row_meas.is_empty() {
                VisualKind::Table
            } else if !col_meas.is_empty() && !row_meas.is_empty() {
                VisualKind::Scatter
            } else if date_dim {
                VisualKind::Line
            } else if horizontal {
                VisualKind::StackedBar
            } else {
                VisualKind::StackedColumn
            }
        }
    };
    v.kind = kind;
    let dims: Vec<FieldUse> = if horizontal { row_dims.iter().chain(col_dims.iter()).cloned().collect() } else { col_dims.iter().chain(row_dims.iter()).cloned().collect() };
    let meas: Vec<FieldUse> = col_meas.iter().chain(row_meas.iter()).cloned().collect();
    // Keep the shelves; rebuild the derived wells.
    let keep: Vec<(String, Vec<FieldUse>)> = v.wells.iter().filter(|(k, _)| k.starts_with("ws.")).map(|(k, f)| (k.clone(), f.clone())).collect();
    v.wells.clear();
    for (k, f) in keep {
        v.wells.insert(k, f);
    }
    match kind {
        VisualKind::Scatter => {
            v.set("x", col_meas.into_iter().take(1).collect());
            v.set("y", row_meas.into_iter().take(1).collect());
            v.set("details", dims.into_iter().chain(v.marks.detail.clone()).collect());
            v.set("legend", v.marks.color.iter().filter(|f| !f.is_value()).cloned().collect());
            v.set("size", v.marks.size.iter().filter(|f| f.is_value()).cloned().collect());
        }
        VisualKind::Table => {
            v.set("columns", dims.into_iter().chain(meas).collect());
        }
        VisualKind::HeatMap => {
            v.set("rows", row_dims);
            v.set("columns", col_dims);
            v.set("values", v.marks.color.iter().filter(|f| f.is_value()).cloned().chain(meas).take(1).collect());
        }
        VisualKind::Pie => {
            v.set("legend", v.marks.color.iter().filter(|f| !f.is_value()).cloned().chain(dims).take(1).collect());
            v.set("values", v.marks.size.iter().filter(|f| f.is_value()).cloned().chain(meas).take(1).collect());
        }
        _ => {
            v.set("axis", dims);
            v.set("values", meas);
            v.set("legend", v.marks.color.iter().filter(|f| !f.is_value()).cloned().take(1).collect());
        }
    }
    let tips: Vec<FieldUse> = v.marks.tooltip.iter().chain(v.marks.label.iter()).filter(|f| f.is_value()).cloned().collect();
    if !tips.is_empty() && kind != VisualKind::Table {
        v.set("tooltips", tips);
    }
    if !v.marks.label.is_empty() {
        // Labels on: data labels.
    }
}

fn shelf(app: &mut VizApp, ui: &mut Ui, t: &Tokens, id: u64, role: &str, label: &str) {
    ui.horizontal(|ui| {
        ui.add_sized([64.0, 24.0], egui::Label::new(RichText::new(label).strong().size(11.5)));
        let fields = app.project.element(id).and_then(|(_, e)| e.visual()).map(|v| v.well(role).to_vec()).unwrap_or_default();
        let frame = egui::Frame::new().fill(t.panel_alt).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(4.0).inner_margin(egui::Margin::symmetric(4, 2));
        let (_, payload) = ui.dnd_drop_zone::<DragField, ()>(frame, |ui| {
            ui.set_min_width(ui.available_width());
            ui.set_min_height(22.0);
            ui.horizontal(|ui| {
                if fields.is_empty() {
                    ui.label(RichText::new("Drop fields here").size(11.0).color(t.muted));
                }
                let mut remove = None;
                for (i, f) in fields.iter().enumerate() {
                    let pill = if f.is_value() { t.good } else { t.accent };
                    let r = ui.add(egui::Button::new(RichText::new(pill_label(f)).color(egui::Color32::WHITE).size(11.5)).fill(pill).corner_radius(10.0));
                    r.context_menu(|ui| {
                        if !f.field.is_measure() {
                            for a in [Agg::None, Agg::Sum, Agg::Average, Agg::Count, Agg::DistinctCount, Agg::Min, Agg::Max, Agg::Median] {
                                if ui.button(if a == Agg::None { "Dimension".to_string() } else { format!("Measure ({})", a.label()) }).clicked() {
                                    set_agg(app, id, role, i, a);
                                    ui.close();
                                }
                            }
                        }
                        if ui.button("Remove").clicked() {
                            remove = Some(i);
                            ui.close();
                        }
                    });
                    if r.double_clicked() {
                        remove = Some(i);
                    }
                }
                if let Some(i) = remove {
                    edit_shelf(app, id, role, |w| {
                        if i < w.len() {
                            w.remove(i);
                        }
                    });
                }
            });
        });
        if let Some(p) = payload {
            let f = p.0.clone();
            let mut fu = FieldUse::new(f.clone());
            if let FieldRef::Column { table, column } = &f
                && app.built.col_type(table, column).is_numeric()
                && !column.to_lowercase().ends_with("id")
            {
                fu.agg = vizcraft_core::state::default_agg(&app.project, &app.built, table, column);
            }
            edit_shelf(app, id, role, |w| w.push(fu));
        }
    });
}

fn pill_label(f: &FieldUse) -> String {
    match (&f.field, f.agg) {
        (FieldRef::Measure { name, .. }, _) => name.clone(),
        (_, Agg::None) => f.field.label(),
        (_, a) => format!("{}({})", format!("{a:?}").to_uppercase(), f.field.label()),
    }
}

fn set_agg(app: &mut VizApp, id: u64, role: &str, i: usize, a: Agg) {
    edit_shelf(app, id, role, |w| {
        if let Some(f) = w.get_mut(i) {
            f.agg = a;
        }
    });
}

fn edit_shelf(app: &mut VizApp, id: u64, role: &str, f: impl FnOnce(&mut Vec<FieldUse>)) {
    app.checkpoint("Shelf");
    let snapshot = app.project.element(id).and_then(|(_, e)| e.visual().cloned());
    let Some(mut v) = snapshot else { return };
    match role {
        "m.color" => f(&mut v.marks.color),
        "m.size" => f(&mut v.marks.size),
        "m.label" => f(&mut v.marks.label),
        "m.detail" => f(&mut v.marks.detail),
        "m.tooltip" => f(&mut v.marks.tooltip),
        r => f(v.well_mut(r)),
    }
    apply_shelves(app, &mut v);
    if let Some(vv) = app.project.element_mut(id).and_then(|e| e.visual_mut()) {
        *vv = v;
    }
    app.report.render_cache.clear();
}

/// The shelves strip above a worksheet's canvas.
pub fn shelves(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let Some(page) = app.current_page().cloned() else { return };
    let Some(id) = page.worksheet_visual.or_else(|| page.elements.iter().find(|e| e.visual().is_some()).map(|e| e.id)) else { return };
    egui::Panel::top("ws-shelves").frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::same(6)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        shelf(app, ui, t, id, "ws.columns", "Columns");
        shelf(app, ui, t, id, "ws.rows", "Rows");
    });
    // Keep the visual filling the page.
    if let Some(pg) = app.current_page_mut() {
        let (w, h) = (pg.width, pg.height);
        if let Some(e) = pg.element_mut(id)
            && (e.x != 0.0 || e.y != 0.0 || e.w != w || e.h != h)
        {
            e.x = 0.0;
            e.y = 0.0;
            e.w = w;
            e.h = h;
        }
    }
}

/// The Marks card and Show Me, shown at the left of a worksheet.
pub fn marks_card(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let Some(page) = app.current_page().cloned() else { return };
    let Some(id) = page.worksheet_visual.or_else(|| page.elements.iter().find(|e| e.visual().is_some()).map(|e| e.id)) else { return };
    let Some(v) = app.project.element(id).and_then(|(_, e)| e.visual().cloned()) else { return };
    ui.label(RichText::new("Marks").strong());
    let mut mark = if v.marks.mark.is_empty() { "Automatic".to_string() } else { v.marks.mark.clone() };
    egui::ComboBox::from_id_salt("mark-type").selected_text(&mark).width(ui.available_width() - 8.0).show_ui(ui, |ui| {
        for m in MARKS {
            ui.selectable_value(&mut mark, m.to_string(), *m);
        }
    });
    if mark != v.marks.mark && !(mark == "Automatic" && v.marks.mark.is_empty()) {
        app.checkpoint("Mark type");
        let mut v2 = v.clone();
        v2.marks.mark = mark;
        apply_shelves(app, &mut v2);
        if let Some(vv) = app.project.element_mut(id).and_then(|e| e.visual_mut()) {
            *vv = v2;
        }
        app.report.render_cache.clear();
    }
    for (role, label, fields) in [("m.color", "Color", &v.marks.color), ("m.size", "Size", &v.marks.size), ("m.label", "Label", &v.marks.label), ("m.detail", "Detail", &v.marks.detail), ("m.tooltip", "Tooltip", &v.marks.tooltip)] {
        let frame = egui::Frame::new().fill(t.panel_alt).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(4.0).inner_margin(egui::Margin::same(4));
        let (_, payload) = ui.dnd_drop_zone::<DragField, ()>(frame, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(label).strong().size(11.0));
            let mut rm = None;
            for (i, f) in fields.iter().enumerate() {
                if ui.add(egui::Button::new(RichText::new(pill_label(f)).size(11.0)).small()).on_hover_text("Click to remove").clicked() {
                    rm = Some(i);
                }
            }
            if let Some(i) = rm {
                edit_shelf(app, id, role, |w| {
                    w.remove(i);
                });
            }
        });
        if let Some(p) = payload {
            let mut fu = FieldUse::new(p.0.clone());
            if let FieldRef::Column { table, column } = &p.0
                && matches!(role, "m.size" | "m.label" | "m.tooltip")
                && app.built.col_type(table, column).is_numeric()
            {
                fu.agg = vizcraft_core::state::default_agg(&app.project, &app.built, table, column);
            }
            edit_shelf(app, id, role, |w| w.push(fu));
        }
    }
    ui.add_space(8.0);
    widgets::section(ui, "showme", "Show Me", true, |ui| {
        let per_row = 5;
        for chunk in [
            VisualKind::Table,
            VisualKind::HeatMap,
            VisualKind::HighlightTable,
            VisualKind::Map,
            VisualKind::Pie,
            VisualKind::StackedBar,
            VisualKind::ClusteredBar,
            VisualKind::StackedColumn,
            VisualKind::ClusteredColumn,
            VisualKind::Line,
            VisualKind::Area,
            VisualKind::DualAxis,
            VisualKind::Scatter,
            VisualKind::Histogram,
            VisualKind::BoxPlot,
            VisualKind::Gantt,
            VisualKind::Bullet,
            VisualKind::PackedBubbles,
            VisualKind::Treemap,
            VisualKind::WordCloud,
        ]
        .chunks(per_row)
        {
            ui.horizontal(|ui| {
                for k in chunk {
                    let (r, resp) = ui.allocate_exact_size(egui::vec2(34.0, 30.0), egui::Sense::click());
                    if v.kind == *k {
                        ui.painter().rect_filled(r, 4.0, t.accent_soft);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, 4.0, t.panel_alt);
                    }
                    crate::icons::visual(ui.painter(), r.shrink(6.0), *k, t.accent);
                    if resp.on_hover_text(k.label()).clicked() {
                        app.checkpoint("Show Me");
                        if let Some(vv) = app.project.element_mut(id).and_then(|e| e.visual_mut()) {
                            vv.kind = *k;
                        }
                        app.report.render_cache.clear();
                    }
                }
            });
        }
    });
}

/// Sets up a new worksheet's shelves from an existing visual's wells.
pub fn shelves_from_wells(v: &mut VisualSpec) {
    if !v.well("ws.columns").is_empty() || !v.well("ws.rows").is_empty() {
        return;
    }
    let horizontal = matches!(v.kind, VisualKind::StackedBar | VisualKind::ClusteredBar | VisualKind::StackedBar100);
    let dims: Vec<FieldUse> = ["axis", "rows", "location", "field", "details"].iter().flat_map(|r| v.well(r).to_vec()).filter(|f| !f.is_value()).collect();
    let meas: Vec<FieldUse> = ["values", "y", "x", "columns"].iter().flat_map(|r| v.well(r).to_vec()).filter(|f| f.is_value()).collect();
    if horizontal {
        v.set("ws.rows", dims);
        v.set("ws.columns", meas);
    } else {
        v.set("ws.columns", dims);
        v.set("ws.rows", meas);
    }
    v.marks.color = v.well("legend").to_vec();
}
