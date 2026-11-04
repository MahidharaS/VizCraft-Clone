//! The start screen shown for an empty project: the brand, quick starts, recent projects and a
//! Copilot prompt. Recent projects live in `~/.vizardcraft/recent.json`.

use std::path::PathBuf;

use egui::{Align2, Color32, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::app::{VizApp, View};
use crate::theme::Tokens;

fn recent_path() -> Option<PathBuf> {
    vizardcraft_ai::installed::home().map(|h| h.join(".vizardcraft").join("recent.json"))
}

/// Recently opened or saved projects (newest first, existing files only).
pub fn recent() -> Vec<PathBuf> {
    let Some(p) = recent_path() else { return vec![] };
    let list: Vec<String> = std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    list.into_iter().map(PathBuf::from).filter(|p| p.exists()).take(8).collect()
}

/// Remembers a project path at the top of the recent list.
pub fn remember(path: &std::path::Path) {
    let Some(rp) = recent_path() else { return };
    let mut list: Vec<String> = std::fs::read_to_string(&rp).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let s = path.to_string_lossy().to_string();
    list.retain(|x| *x != s);
    list.insert(0, s);
    list.truncate(20);
    if let Some(d) = rp.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(rp, serde_json::to_string_pretty(&list).unwrap_or_default());
}

/// True when the project has nothing in it yet (no data, one empty page).
pub fn is_empty(app: &VizApp) -> bool {
    app.project.model.tables.is_empty()
        && app.project.queries.queries.is_empty()
        && app.project.sheets.is_empty()
        && app.project.report.pages.len() <= 1
        && app.project.report.pages.iter().all(|p| p.elements.is_empty())
        && app.project.studio.is_empty()
}

/// The VizardCraft mark (layered stack with bars and the AI spark), from `assets/brand`.
pub fn logo(p: &egui::Painter, r: Rect) {
    let tex = logo_texture(p.ctx());
    p.image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
}

fn logo_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let id = egui::Id::new("vizardcraft-brand-mark");
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return t;
    }
    let bytes = include_bytes!("../../../assets/brand/vizardcraft-mark-256.png");
    let img = image::load_from_memory(bytes).map(|i| i.to_rgba8()).unwrap_or_else(|_| image::RgbaImage::new(1, 1));
    let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
    let t = ctx.load_texture("vizardcraft-mark", ci, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    t
}

fn tile(ui: &mut Ui, t: &Tokens, icon: &str, title: &str, sub: &str, w: f32) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(w, 74.0), Sense::click());
    let p = ui.painter();
    let hover = resp.hovered();
    p.rect_filled(r, 10.0, if hover { t.accent_soft } else { t.panel });
    p.rect_stroke(r, 10.0, Stroke::new(1.0, if hover { t.accent } else { t.border }), StrokeKind::Inside);
    let ir = Rect::from_center_size(pos2(r.left() + 30.0, r.center().y), vec2(26.0, 26.0));
    p.circle_filled(ir.center(), 20.0, t.accent_soft);
    crate::icons::paint(p, ir.shrink(3.0), icon, t.accent);
    p.text(pos2(r.left() + 60.0, r.center().y - 9.0), Align2::LEFT_CENTER, title, FontId::proportional(13.5), t.text);
    p.text(pos2(r.left() + 60.0, r.center().y + 10.0), Align2::LEFT_CENTER, sub, FontId::proportional(11.0), t.muted);
    if hover {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

pub fn show(app: &mut VizApp, ui: &mut Ui) {
    let t = app.tokens();
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let full = ui.available_width();
            let w = full.min(980.0);
            let pad = ((full - w) / 2.0).max(16.0);
            ui.add_space(36.0);
            ui.horizontal(|ui| {
                ui.add_space(pad);
                let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
                logo(ui.painter(), r);
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.label(RichText::new("Vizard").size(28.0).strong());
                        ui.label(RichText::new("Craft").size(28.0).strong().color(Color32::from_rgb(0x8A, 0x2B, 0xE2)));
                    });
                    ui.label(RichText::new("Analytics studio: data prep, models, reports, ML and AI in one place").size(13.0).color(t.muted));
                });
            });
            ui.add_space(22.0);
            // Copilot prompt.
            ui.horizontal(|ui| {
                ui.add_space(pad);
                egui::Frame::new().fill(t.panel).stroke(Stroke::new(1.0, t.border)).corner_radius(12.0).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
                    ui.set_width(w - 28.0);
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                        crate::icons::paint(ui.painter(), r, "ai", t.accent);
                        let resp = ui.add(egui::TextEdit::singleline(&mut app.welcome_prompt).hint_text("Tell Copilot what to build, e.g. \"load sales.csv and make a sales dashboard by region\"").frame(egui::Frame::NONE).desired_width(w - 160.0).font(FontId::proportional(14.0)));
                        let go = ui.add(egui::Button::new(RichText::new("Ask Copilot").color(Color32::WHITE)).fill(t.accent).corner_radius(8.0)).clicked() || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                        if go && !app.welcome_prompt.trim().is_empty() {
                            let q = std::mem::take(&mut app.welcome_prompt);
                            app.ai.copilot_open = true;
                            crate::ai::copilot::ask(app, q);
                        }
                    });
                });
            });
            ui.add_space(22.0);
            let col = (w - 24.0) / 3.0;
            let section = |ui: &mut Ui, s: &str| {
                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    ui.label(RichText::new(s).size(12.5).strong().color(t.muted));
                });
                ui.add_space(6.0);
            };
            section(ui, "START");
            let starts: [(&str, &str, &str, &str); 6] = [
                ("sheet", "Excel workbook", "Import sheets and tables", "excel"),
                ("data", "Text / CSV", "Import a delimited file", "csv"),
                ("sql", "Database", "PostgreSQL, MySQL, SQLite…", "postgresql"),
                ("service", "Web / API", "JSON, REST, OData", "web"),
                ("table", "Enter data", "Type or paste a table", "enter"),
                ("report", "Retail sample", "Explore a finished project", "sample"),
            ];
            for row in starts.chunks(3) {
                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    for (icon, title, sub, key) in row {
                        if tile(ui, &t, icon, title, sub, col).clicked() {
                            if *key == "sample" {
                                app.load_project(vizardcraft_core::sample::sales(), None);
                            } else {
                                crate::pq::get_data(app, key);
                            }
                        }
                        ui.add_space(12.0);
                    }
                });
                ui.add_space(12.0);
            }
            section(ui, "MORE WAYS TO START");
            let more: [(&str, &str, &str, u8); 3] = [("studio", "Machine learning", "Studio: train, evaluate, deploy", 0), ("sql", "SQL", "Query files with DuckDB", 1), ("open", "Open project", ".vizc file or .vizp Git folder", 2)];
            ui.horizontal(|ui| {
                ui.add_space(pad);
                for (icon, title, sub, k) in more {
                    if tile(ui, &t, icon, title, sub, col).clicked() {
                        match k {
                            0 => {
                                app.load_project(vizardcraft_core::sample::sales(), None);
                                crate::studio::new_workflow(app, Some("high_margin"));
                                app.view = View::Studio;
                            }
                            1 => app.view = View::Sql,
                            _ => app.open_dialog(),
                        }
                    }
                    ui.add_space(12.0);
                }
            });
            ui.add_space(20.0);
            let rec = recent();
            if !rec.is_empty() {
                section(ui, "RECENT");
                let mut open = None;
                for p in &rec {
                    ui.horizontal(|ui| {
                        ui.add_space(pad);
                        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        let folder = p.extension().is_some_and(|e| e == vizardcraft_core::project_folder::EXT);
                        let (r, resp) = ui.allocate_exact_size(vec2(w, 34.0), Sense::click());
                        let pnt = ui.painter();
                        if resp.hovered() {
                            pnt.rect_filled(r, 6.0, t.panel);
                        }
                        crate::icons::paint(pnt, Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(16.0, 16.0)), if folder { "git" } else { "report" }, t.accent);
                        pnt.text(pos2(r.left() + 34.0, r.center().y), Align2::LEFT_CENTER, &name, FontId::proportional(13.0), t.text);
                        let dir = p.parent().map(|d| d.display().to_string()).unwrap_or_default();
                        pnt.text(pos2(r.left() + 280.0, r.center().y), Align2::LEFT_CENTER, dir, FontId::proportional(11.0), t.muted);
                        if resp.clicked() {
                            open = Some(p.clone());
                        }
                    });
                }
                if let Some(p) = open {
                    app.open_path(p);
                }
            }
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.add_space(pad);
                if ui.link("Start with a blank report").clicked() {
                    app.welcome_dismissed = true;
                }
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.label(RichText::new("Tip: Ctrl/⌘ + I opens Copilot anywhere. Save as folder (.vizp) to keep the project in Git and let agents edit it.").size(11.0).color(t.muted));
            });
        });
    });
}
