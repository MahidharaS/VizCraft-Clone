//! Context menus for elements.

use egui::Ui;
use vizcraft_core::doc::*;

use crate::app::VizApp;

pub fn element_menu(app: &mut VizApp, ui: &mut Ui) {
    let one = app.selected.first().copied();
    let is_visual = one.and_then(|id| app.project.element(id)).is_some_and(|(_, e)| e.visual().is_some());
    if ui.button("Copy").clicked() {
        super::clipboard(app, ui.ctx(), "copy");
        ui.close();
    }
    if ui.button("Paste").clicked() {
        super::clipboard(app, ui.ctx(), "paste");
        ui.close();
    }
    if ui.button("Duplicate").clicked() {
        super::duplicate(app);
        ui.close();
    }
    if ui.button("Delete").clicked() {
        super::delete_selected(app);
        ui.close();
    }
    ui.separator();
    if ui.button("Group").clicked() {
        super::arrange::group(app);
        ui.close();
    }
    if ui.button("Ungroup").clicked() {
        super::arrange::ungroup(app);
        ui.close();
    }
    ui.menu_button("Order", |ui| {
        for (label, op) in [("Bring to front", "front"), ("Bring forward", "forward"), ("Send backward", "backward"), ("Send to back", "back")] {
            if ui.button(label).clicked() {
                super::arrange::order(app, op);
                ui.close();
            }
        }
    });
    ui.menu_button("Align", |ui| {
        for (label, op) in [("Left", "left"), ("Center", "center"), ("Right", "right"), ("Top", "top"), ("Middle", "middle"), ("Bottom", "bottom"), ("Distribute horizontally", "dist-h"), ("Distribute vertically", "dist-v")] {
            if ui.button(label).clicked() {
                super::arrange::align(app, op);
                ui.close();
            }
        }
    });
    if let Some(id) = one {
        let locked = app.project.element(id).is_some_and(|(_, e)| e.locked);
        if ui.button(if locked { "Unlock" } else { "Lock" }).clicked() {
            app.checkpoint("Lock");
            if let Some(e) = app.project.element_mut(id) {
                e.locked = !locked;
            }
            ui.close();
        }
        if ui.button("Hide").clicked() {
            app.checkpoint("Hide");
            if let Some(e) = app.project.element_mut(id) {
                e.hidden = true;
            }
            ui.close();
        }
    }
    if is_visual && let Some(id) = one {
        ui.separator();
        if ui.button("Open as sheet  ↗").clicked() {
            super::open_as_sheet(app, id);
            ui.close();
        }
        if ui.button("Show as a table").clicked() {
            app.report.show_as_table = Some(id);
            ui.close();
        }
        if ui.button("Spotlight").clicked() {
            app.rt.spotlight = if app.rt.spotlight == Some(id) { None } else { Some(id) };
            ui.close();
        }
        if ui.button("Focus mode").clicked() {
            app.rt.focus = Some(id);
            ui.close();
        }
        if ui.button("Export data (CSV)").clicked() {
            super::export_data(app, id);
            ui.close();
        }
        ui.menu_button("Sort by", |ui| {
            let labels: Vec<String> = app.report.render_cache.get(&(id, app.page)).and_then(|(_, r)| r.data.clone()).map(|d| d.cols.iter().map(|c| c.label.clone()).collect()).unwrap_or_default();
            for l in labels {
                if ui.button(&l).clicked() {
                    super::actions::visual_action(app, id, &format!("sort:{l}"), egui::Modifiers::NONE);
                    ui.close();
                }
            }
        });
        ui.menu_button("Change visual type", |ui| {
            for k in VisualKind::ALL {
                if ui.button(k.label()).clicked() {
                    app.checkpoint("Change visual");
                    if let Some(v) = app.project.element_mut(id).and_then(|e| e.visual_mut()) {
                        v.kind = *k;
                    }
                    app.report.render_cache.clear();
                    ui.close();
                }
            }
        });
        if app.current_page().is_some_and(|p| p.kind == PageKind::Worksheet)
            && ui.button("Add to dashboard").clicked()
        {
            super::dashboard::add_sheet_to_new_or_existing(app, id);
            ui.close();
        }
    }
}
