//! Bookmarks: capture and apply the report's view state (page, filters, slicers, cross-filter
//! selection, drill, visibility, parameters, focus), plus the Bookmarks pane.

use egui::{RichText, Ui};
use vizardcraft_core::doc::*;

use crate::app::VizApp;
use crate::theme::Tokens;

pub fn capture(app: &VizApp) -> ViewState {
    let p = &app.project;
    let mut s = ViewState::default();
    for f in &p.report.filters {
        s.filters.insert(f.id, f.kind.clone());
    }
    for pg in &p.report.pages {
        for f in &pg.filters {
            s.filters.insert(f.id, f.kind.clone());
        }
        for e in &pg.elements {
            s.hidden.insert(e.id, e.hidden);
            if let Some(v) = e.visual() {
                for f in &v.filters {
                    s.filters.insert(f.id, f.kind.clone());
                }
                if v.kind == VisualKind::Slicer {
                    s.slicers.insert(e.id, (v.selection.clone(), v.range));
                }
                s.drill.insert(e.id, v.drill.clone());
            }
        }
    }
    s.selection = app.rt.selection.clone();
    s.parameters = p.model.parameters.iter().map(|x| (x.name.clone(), x.current.clone())).collect();
    s.focus = app.rt.focus;
    s
}

pub fn apply_state(app: &mut VizApp, s: &ViewState, data: bool, display: bool, only: Option<&[u64]>) {
    let allowed = |id: u64| only.is_none_or(|o| o.contains(&id));
    let p = &mut app.project;
    if data {
        for f in p.report.filters.iter_mut() {
            if let Some(k) = s.filters.get(&f.id) {
                f.kind = k.clone();
            }
        }
    }
    for pg in p.report.pages.iter_mut() {
        if data {
            for f in pg.filters.iter_mut() {
                if let Some(k) = s.filters.get(&f.id) {
                    f.kind = k.clone();
                }
            }
        }
        for e in pg.elements.iter_mut() {
            if !allowed(e.id) {
                continue;
            }
            if display && let Some(h) = s.hidden.get(&e.id) {
                e.hidden = *h;
            }
            let id = e.id;
            if let Some(v) = e.visual_mut() {
                if data {
                    for f in v.filters.iter_mut() {
                        if let Some(k) = s.filters.get(&f.id) {
                            f.kind = k.clone();
                        }
                    }
                    if let Some((sel, range)) = s.slicers.get(&id) {
                        v.selection = sel.clone();
                        v.range = *range;
                    }
                }
                if display && let Some(d) = s.drill.get(&id) {
                    v.drill = d.clone();
                }
            }
        }
    }
    if data {
        for prm in p.model.parameters.iter_mut() {
            if let Some(v) = s.parameters.get(&prm.name) {
                prm.current = v.clone();
            }
        }
        app.rt.selection = s.selection.clone();
        app.mark_model_dirty();
    }
    if display {
        app.rt.focus = s.focus;
    }
    app.report.render_cache.clear();
}

pub fn apply(app: &mut VizApp, b: &Bookmark) {
    if b.capture_current_page && app.project.page(b.page).is_some() {
        app.go_to_page(b.page);
    }
    let only = if b.all_visuals { None } else { Some(b.selected_visuals.as_slice()) };
    apply_state(app, &b.state, b.capture_data, b.capture_display, only);
    app.report.last_bookmark = Some(b.id);
}

pub fn add(app: &mut VizApp) {
    app.checkpoint("Add bookmark");
    let id = app.project.next_id();
    let n = app.project.report.bookmarks.len() + 1;
    let state = capture(app);
    app.project.report.bookmarks.push(Bookmark {
        id,
        name: format!("Bookmark {n}"),
        group: String::new(),
        page: app.page,
        state,
        capture_data: true,
        capture_display: true,
        capture_current_page: true,
        all_visuals: true,
        selected_visuals: vec![],
    });
    app.report.last_bookmark = Some(id);
}

pub fn pane(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    if crate::widgets::pane_header(ui, t, "Bookmarks") {
        app.panes.bookmarks = false;
    }
    ui.horizontal(|ui| {
        if ui.button("+ Add").on_hover_text("Capture the current view (page, filters, slicers, visibility)").clicked() {
            add(app);
        }
        if ui.button("View").on_hover_text("Slideshow through bookmarks").clicked() {
            app.report.reading = !app.report.reading;
        }
        if ui.button("Add navigator").on_hover_text("Insert a bookmark navigator").clicked() {
            super::insert_button(app, "Bookmark navigator");
        }
    });
    ui.separator();
    let mut del = None;
    let mut up = None;
    let bms = app.project.report.bookmarks.clone();
    if bms.is_empty() {
        ui.label(RichText::new("No bookmarks yet. Set up filters, slicers and visibility, then Add.").color(t.muted).size(11.0));
    }
    egui::ScrollArea::vertical().id_salt("bm-scroll").show(ui, |ui| {
        for (i, b) in bms.iter().enumerate() {
            let active = app.report.last_bookmark == Some(b.id);
            egui::Frame::new().fill(if active { t.accent_soft } else { t.panel }).corner_radius(4.0).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    crate::icons::paint(ui.painter(), egui::Rect::from_min_size(ui.cursor().min, egui::vec2(16.0, 16.0)), "bookmark", t.accent);
                    ui.add_space(18.0);
                    if ui.add(egui::Button::new(&b.name).frame(false)).clicked() {
                        apply(app, b);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.menu_button("⋯", |ui| {
                            if ui.button("Update").clicked() {
                                app.checkpoint("Update bookmark");
                                let st = capture(app);
                                let page = app.page;
                                if let Some(bm) = app.project.report.bookmarks.iter_mut().find(|x| x.id == b.id) {
                                    bm.state = st;
                                    bm.page = page;
                                }
                                ui.close();
                            }
                            let mut bm = b.clone();
                            let mut changed = false;
                            ui.label("Name");
                            changed |= ui.text_edit_singleline(&mut bm.name).changed();
                            ui.label("Group");
                            changed |= ui.text_edit_singleline(&mut bm.group).changed();
                            changed |= ui.checkbox(&mut bm.capture_data, "Data (filters, slicers)").changed();
                            changed |= ui.checkbox(&mut bm.capture_display, "Display (visibility, drill)").changed();
                            changed |= ui.checkbox(&mut bm.capture_current_page, "Current page").changed();
                            changed |= ui.checkbox(&mut bm.all_visuals, "All visuals").changed();
                            if !bm.all_visuals && ui.button("Use selected visuals").clicked() {
                                bm.selected_visuals = app.selected.clone();
                                changed = true;
                            }
                            if changed && let Some(x) = app.project.report.bookmarks.iter_mut().find(|x| x.id == b.id) {
                                *x = bm;
                                app.dirty = true;
                            }
                            if ui.button("Move up").clicked() {
                                up = Some(i);
                                ui.close();
                            }
                            if ui.button("Delete").clicked() {
                                del = Some(b.id);
                                ui.close();
                            }
                        });
                    });
                });
                if !b.group.is_empty() {
                    ui.label(RichText::new(format!("Group: {}", b.group)).size(10.0).color(t.muted));
                }
            });
        }
    });
    if let Some(id) = del {
        app.checkpoint("Delete bookmark");
        app.project.report.bookmarks.retain(|b| b.id != id);
    }
    if let Some(i) = up
        && i > 0
    {
        app.project.report.bookmarks.swap(i, i - 1);
    }
}
