//! Selection (layers) pane, Sync slicers pane and Performance analyzer.

use egui::{RichText, Ui};
use vizcraft_core::doc::*;

use crate::app::VizApp;
use crate::theme::Tokens;
use crate::widgets;

pub fn selection_pane(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    if widgets::pane_header(ui, t, "Selection (layers)") {
        app.panes.selection = false;
    }
    ui.horizontal(|ui| {
        if ui.small_button("Show all").clicked() {
            app.checkpoint("Show all");
            if let Some(p) = app.current_page_mut() {
                for e in p.elements.iter_mut() {
                    e.hidden = false;
                }
            }
        }
        if ui.small_button("Hide all").clicked() {
            app.checkpoint("Hide all");
            if let Some(p) = app.current_page_mut() {
                for e in p.elements.iter_mut() {
                    e.hidden = true;
                }
            }
        }
        if ui.small_button("▲").on_hover_text("Bring forward").clicked() {
            super::arrange::order(app, "forward");
        }
        if ui.small_button("▼").on_hover_text("Send backward").clicked() {
            super::arrange::order(app, "backward");
        }
    });
    ui.separator();
    let Some(page) = app.current_page().cloned() else { return };
    egui::ScrollArea::vertical().id_salt("layers").show(ui, |ui| {
        // Top of the z-order first, children indented under their group.
        for e in page.elements.iter().rev().filter(|e| e.parent.is_none()) {
            layer_row(app, ui, t, &page, e, 0);
        }
    });
}

fn layer_row(app: &mut VizApp, ui: &mut Ui, t: &Tokens, page: &Page, e: &Element, depth: usize) {
    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 14.0);
        let eye = if e.hidden { "◌" } else { "◉" };
        if ui.add(egui::Button::new(eye).frame(false)).on_hover_text("Show / hide").clicked() {
            app.checkpoint("Visibility");
            if let Some(x) = app.project.element_mut(e.id) {
                x.hidden = !x.hidden;
            }
        }
        let name = if e.name.is_empty() { e.kind_label() } else { e.name.clone() };
        let sel = app.selected.contains(&e.id);
        let r = ui.selectable_label(sel, RichText::new(format!("{}  {}", if e.kind == ElementKind::Group { "▾" } else { " " }, name)).color(if e.hidden { t.muted } else { t.text }));
        if r.clicked() {
            if ui.input(|i| i.modifiers.shift || i.modifiers.command) {
                if sel {
                    app.selected.retain(|x| *x != e.id);
                } else {
                    app.selected.push(e.id);
                }
            } else {
                app.selected = vec![e.id];
            }
        }
        if e.locked {
            ui.label(RichText::new("locked").size(10.0).color(t.muted));
        }
    });
    if e.kind == ElementKind::Group {
        for k in page.elements.iter().rev().filter(|k| k.parent == Some(e.id)) {
            layer_row(app, ui, t, page, k, depth + 1);
        }
    }
}

pub fn sync_pane(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    if widgets::pane_header(ui, t, "Sync slicers") {
        app.panes.sync_slicers = false;
    }
    let Some(id) = app.selected.first().copied() else {
        ui.label(RichText::new("Select a slicer to sync it across pages.").color(t.muted).size(11.0));
        return;
    };
    let Some((_, el)) = app.project.element(id).map(|(p, e)| (p, e.clone())) else { return };
    let Some(v) = el.visual() else { return };
    if v.kind != VisualKind::Slicer {
        ui.label(RichText::new("Select a slicer.").color(t.muted));
        return;
    }
    let group = el.sync_group.clone().unwrap_or_else(|| v.well("field").first().map(|f| f.label()).unwrap_or_else(|| format!("slicer-{id}")));
    ui.label(RichText::new(format!("Group: {group}")).strong());
    ui.label(RichText::new("Sync = filter that page too; Visible = place a copy of the slicer on it.").size(10.5).color(t.muted));
    let pages: Vec<(u64, String, bool, bool)> = app
        .project
        .report
        .pages
        .iter()
        .map(|p| {
            let synced = p.elements.iter().any(|e| e.sync_group.as_deref() == Some(group.as_str()));
            let visible = p.elements.iter().any(|e| e.sync_group.as_deref() == Some(group.as_str()) && !e.hidden);
            (p.id, p.name.clone(), synced, visible)
        })
        .collect();
    egui::Grid::new("sync").num_columns(3).show(ui, |ui| {
        ui.label(RichText::new("Page").strong());
        ui.label(RichText::new("Sync").strong());
        ui.label(RichText::new("Visible").strong());
        ui.end_row();
        for (pid, name, synced, visible) in pages {
            ui.label(name);
            let mut s = synced;
            let mut vis = visible;
            let sc = ui.checkbox(&mut s, "").changed();
            let vc = ui.checkbox(&mut vis, "").changed();
            ui.end_row();
            if sc || vc {
                app.checkpoint("Sync slicers");
                if let Some(e) = app.project.element_mut(id) {
                    e.sync_group = Some(group.clone());
                }
                let template = app.project.element(id).map(|(_, e)| e.clone());
                if let (Some(mut tpl), Some(pg)) = (template, app.project.page_mut(pid)) {
                    let existing = pg.elements.iter().position(|e| e.sync_group.as_deref() == Some(group.as_str()));
                    match (s, existing) {
                        (true, None) => {
                            tpl.id = pid.wrapping_mul(1000).wrapping_add(id % 1000).wrapping_add(7_000_000);
                            tpl.hidden = !vis;
                            pg.elements.push(tpl);
                        }
                        (true, Some(i)) => {
                            if let Some(e) = pg.elements.get_mut(i) {
                                e.hidden = !vis;
                            }
                        }
                        (false, Some(i)) => {
                            if pg.elements.get(i).map(|e| e.id) != Some(id) {
                                pg.elements.remove(i);
                            }
                        }
                        _ => {}
                    }
                }
                app.report.render_cache.clear();
            }
        }
    });
}

pub fn performance_pane(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    if widgets::pane_header(ui, t, "Performance analyzer") {
        app.panes.performance = false;
    }
    ui.horizontal(|ui| {
        if ui.button("Refresh visuals").clicked() {
            app.report.render_cache.clear();
        }
        if ui.button("Refresh data").clicked() {
            app.mark_data_dirty();
        }
    });
    ui.label(RichText::new(format!("Last data refresh: {:.0} ms", app.last_refresh_ms)).size(11.0));
    ui.separator();
    let page = app.page;
    let mut rows: Vec<(String, f64, usize)> = app
        .report
        .render_cache
        .iter()
        .filter(|((_, p), _)| *p == page)
        .map(|((id, _), (_, r))| {
            let name = app.project.element(*id).map(|(_, e)| if e.name.is_empty() { e.kind_label() } else { e.name.clone() }).unwrap_or_default();
            (name, r.millis, r.data.as_ref().map(|d| d.rows.len()).unwrap_or(0))
        })
        .collect();
    rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    egui::Grid::new("perf").striped(true).num_columns(3).show(ui, |ui| {
        ui.label(RichText::new("Visual").strong());
        ui.label(RichText::new("Query + render").strong());
        ui.label(RichText::new("Rows").strong());
        ui.end_row();
        for (n, ms, r) in rows {
            ui.label(n);
            ui.label(format!("{ms:.1} ms"));
            ui.label(r.to_string());
            ui.end_row();
        }
    });
}
