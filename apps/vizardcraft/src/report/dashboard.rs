//! Tableau-style dashboards: a side pane listing sheets and objects to add, tiled/floating
//! layout, and dashboard actions (filter, highlight, URL, go to sheet, change parameter).

use egui::{RichText, Ui};
use vizardcraft_core::doc::*;

use crate::app::VizApp;
use crate::theme::Tokens;

pub fn pane(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    ui.label(RichText::new("Dashboard").strong().size(13.0));
    ui.add_space(4.0);
    ui.label(RichText::new("Sheets").strong());
    let sheets: Vec<(u64, String, Option<Element>)> = app
        .project
        .report
        .pages
        .iter()
        .filter(|p| p.kind == PageKind::Worksheet)
        .map(|p| (p.id, p.name.clone(), p.worksheet_visual.and_then(|w| p.element(w).cloned()).or_else(|| p.elements.iter().find(|e| e.visual().is_some()).cloned())))
        .collect();
    if sheets.is_empty() {
        ui.label(RichText::new("No worksheets yet. Create one with the ↗ icon on any visual, or Insert → New worksheet.").size(10.5).color(t.muted));
    }
    for (pid, name, el) in sheets {
        ui.horizontal(|ui| {
            crate::icons::paint(ui.painter(), egui::Rect::from_min_size(ui.cursor().min, egui::vec2(16.0, 16.0)), "worksheet", t.accent);
            ui.add_space(18.0);
            if ui.button(&name).on_hover_text("Add this sheet to the dashboard").clicked()
                && let Some(e) = el.clone()
            {
                add_sheet(app, &name, e);
            }
            let _ = pid;
        });
    }
    ui.separator();
    ui.label(RichText::new("Objects").strong());
    ui.horizontal_wrapped(|ui| {
        if ui.button("Text").clicked() {
            super::insert_text(app);
        }
        if ui.button("Image").clicked() {
            super::insert_image(app);
        }
        if ui.button("Navigation").clicked() {
            super::insert_button(app, "Page navigator");
        }
        if ui.button("Blank").clicked() {
            super::insert_shape(app, ShapeKind::Rectangle);
        }
        if ui.button("Container").clicked() {
            super::insert_shape(app, ShapeKind::Frame);
        }
        if ui.button("Web link").clicked() {
            super::insert_button(app, "Web");
        }
    });
    ui.separator();
    ui.label(RichText::new("Layout").strong());
    ui.horizontal(|ui| {
        if ui.button("Tile all (tiled)").on_hover_text("Arrange non-floating objects in a grid").clicked() {
            super::arrange::tile(app);
        }
        if ui.button("Make selection floating").clicked() {
            let sel = app.selected.clone();
            app.checkpoint("Floating");
            if let Some(pg) = app.current_page_mut() {
                for e in pg.elements.iter_mut().filter(|e| sel.contains(&e.id)) {
                    e.floating = !e.floating;
                }
            }
        }
    });
    ui.separator();
    ui.label(RichText::new("Actions").strong());
    let actions = app.current_page().map(|p| p.actions.clone()).unwrap_or_default();
    for a in &actions {
        ui.label(format!("{} — {} ({} → {})", a.name, a.kind, a.source.len(), a.targets.len()));
    }
    if ui.button("Dashboard actions…").clicked() {
        app.report.actions_dialog = true;
    }
}

pub fn add_sheet(app: &mut VizApp, name: &str, el: Element) {
    app.checkpoint("Add sheet to dashboard");
    let id = app.project.next_id();
    let Some(pg) = app.current_page_mut() else { return };
    let n = pg.elements.iter().filter(|e| e.visual().is_some()).count();
    let mut e = el;
    e.id = id;
    e.name = name.to_string();
    e.floating = false;
    e.x = 16.0 + (n % 2) as f32 * (pg.width / 2.0);
    e.y = 16.0 + (n / 2) as f32 * 320.0;
    e.w = pg.width / 2.0 - 24.0;
    e.h = 300.0;
    pg.elements.push(e);
    app.selected = vec![id];
}

/// "Add to dashboard" from a worksheet: adds to the first dashboard (or a new one).
pub fn add_sheet_to_new_or_existing(app: &mut VizApp, element: u64) {
    let Some((_, el)) = app.project.element(element).map(|(p, e)| (p, e.clone())) else { return };
    let name = app.current_page().map(|p| p.name.clone()).unwrap_or_default();
    let dash = app.project.report.pages.iter().find(|p| p.kind == PageKind::Dashboard).map(|p| p.id);
    match dash {
        Some(d) => app.go_to_page(d),
        None => super::new_page(app, PageKind::Dashboard),
    }
    add_sheet(app, &name, el);
}

pub fn actions_dialog(app: &mut VizApp, ctx: &egui::Context) {
    if !app.report.actions_dialog {
        return;
    }
    let mut open = true;
    let Some(page) = app.current_page().cloned() else { return };
    let visuals: Vec<(u64, String)> = page.elements.iter().filter(|e| e.visual().is_some()).map(|e| (e.id, if e.name.is_empty() { e.kind_label() } else { e.name.clone() })).collect();
    let pages: Vec<(u64, String)> = app.project.report.pages.iter().map(|p| (p.id, p.name.clone())).collect();
    let params: Vec<String> = app.project.model.parameters.iter().map(|p| p.name.clone()).collect();
    let mut actions = page.actions.clone();
    let mut changed = false;
    egui::Window::new("Dashboard actions").open(&mut open).default_width(620.0).show(ctx, |ui| {
        ui.label(RichText::new("Actions run when marks are selected in a source sheet.").size(11.0));
        let mut remove = None;
        for (i, a) in actions.iter_mut().enumerate() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    changed |= ui.text_edit_singleline(&mut a.name).changed();
                    egui::ComboBox::from_id_salt(("ak", i)).selected_text(&a.kind).show_ui(ui, |ui| {
                        for k in ["Filter", "Highlight", "URL", "Go to Sheet", "Change Parameter"] {
                            changed |= ui.selectable_value(&mut a.kind, k.to_string(), k).changed();
                        }
                    });
                    egui::ComboBox::from_id_salt(("ar", i)).selected_text(&a.run_on).show_ui(ui, |ui| {
                        for k in ["Select", "Hover", "Menu"] {
                            changed |= ui.selectable_value(&mut a.run_on, k.to_string(), k).changed();
                        }
                    });
                    if ui.small_button("✕").clicked() {
                        remove = Some(i);
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Source:");
                    for (id, name) in &visuals {
                        let mut on = a.source.contains(id);
                        if ui.checkbox(&mut on, name).changed() {
                            if on { a.source.push(*id) } else { a.source.retain(|x| x != id) }
                            changed = true;
                        }
                    }
                });
                match a.kind.as_str() {
                    "Filter" | "Highlight" => {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Targets:");
                            for (id, name) in &visuals {
                                let mut on = a.targets.contains(id);
                                if ui.checkbox(&mut on, name).changed() {
                                    if on { a.targets.push(*id) } else { a.targets.retain(|x| x != id) }
                                    changed = true;
                                }
                            }
                        });
                        egui::ComboBox::from_id_salt(("ac", i)).selected_text(&a.on_clear).show_ui(ui, |ui| {
                            for k in ["Keep filtered values", "Show all values", "Exclude all values"] {
                                changed |= ui.selectable_value(&mut a.on_clear, k.to_string(), k).changed();
                            }
                        });
                    }
                    "URL" => {
                        ui.horizontal(|ui| {
                            ui.label("URL (use <Field>):");
                            changed |= ui.text_edit_singleline(&mut a.url).changed();
                        });
                    }
                    "Go to Sheet" => {
                        egui::ComboBox::from_id_salt(("ap", i)).selected_text(pages.iter().find(|p| Some(p.0) == a.target_page).map(|p| p.1.clone()).unwrap_or_else(|| "(choose)".into())).show_ui(ui, |ui| {
                            for (pid, name) in &pages {
                                if ui.selectable_label(a.target_page == Some(*pid), name).clicked() {
                                    a.target_page = Some(*pid);
                                    changed = true;
                                }
                            }
                        });
                    }
                    "Change Parameter" => {
                        egui::ComboBox::from_id_salt(("apar", i)).selected_text(a.parameter.clone().unwrap_or_else(|| "(choose)".into())).show_ui(ui, |ui| {
                            for p in &params {
                                if ui.selectable_label(a.parameter.as_ref() == Some(p), p).clicked() {
                                    a.parameter = Some(p.clone());
                                    changed = true;
                                }
                            }
                        });
                    }
                    _ => {}
                }
            });
        }
        if let Some(i) = remove {
            actions.remove(i);
            changed = true;
        }
        if ui.button("+ Add action").clicked() {
            actions.push(DashboardAction {
                id: actions.len() as u64 + 1,
                name: format!("Action {}", actions.len() + 1),
                kind: "Filter".into(),
                source: visuals.first().map(|v| vec![v.0]).unwrap_or_default(),
                targets: visuals.iter().skip(1).map(|v| v.0).collect(),
                run_on: "Select".into(),
                on_clear: "Show all values".into(),
                url: String::new(),
                target_page: None,
                parameter: None,
                source_field: None,
            });
            changed = true;
        }
    });
    if changed {
        app.checkpoint("Dashboard actions");
        if let Some(pg) = app.current_page_mut() {
            pg.actions = actions;
        }
    }
    if !open {
        app.report.actions_dialog = false;
    }
}
