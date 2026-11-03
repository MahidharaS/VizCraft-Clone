//! Page tabs (report pages, Tableau worksheets and dashboards), page settings and the mobile
//! layout designer.

use egui::{Color32, RichText, Sense, Stroke, StrokeKind, Ui, vec2};
use vizcraft_core::doc::*;

use crate::app::VizApp;
use crate::theme::Tokens;
use crate::widgets;

fn kind_icon(k: PageKind) -> &'static str {
    match k {
        PageKind::Worksheet => "worksheet",
        PageKind::Dashboard => "dashboard",
        PageKind::Tooltip => "text",
        PageKind::Drillthrough => "filter",
        PageKind::Report => "report",
    }
}

pub fn tabs(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    egui::Panel::bottom("page-tabs").exact_size(34.0).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::symmetric(8, 4)).stroke(Stroke::new(1.0, t.border))).show(ui, |ui| {
        egui::ScrollArea::horizontal().id_salt("tabs-scroll").show(ui, |ui| {
            ui.horizontal(|ui| {
                let pages: Vec<(u64, String, PageKind, bool)> = app.project.report.pages.iter().map(|p| (p.id, p.name.clone(), p.kind, p.hidden)).collect();
                let mut drag_to: Option<(usize, usize)> = None;
                for (i, (id, name, kind, hidden)) in pages.iter().enumerate() {
                    let active = *id == app.page;
                    let w = crate::widgets::text_w(ui, name) + 40.0;
                    let (r, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click_and_drag());
                    let fill = if active { t.accent_soft } else if resp.hovered() { t.panel_alt } else { Color32::TRANSPARENT };
                    ui.painter().rect(r, 4.0, fill, if active { Stroke::new(1.0, t.accent) } else { Stroke::NONE }, StrokeKind::Inside);
                    crate::icons::paint(ui.painter(), egui::Rect::from_min_size(r.min + vec2(6.0, 4.0), vec2(16.0, 16.0)), kind_icon(*kind), if active { t.accent } else { t.muted });
                    let txt = if *hidden { format!("{name} (hidden)") } else { name.clone() };
                    ui.painter().text(r.min + vec2(26.0, 12.0), egui::Align2::LEFT_CENTER, txt, egui::FontId::proportional(12.0), if active { t.accent } else { t.text });
                    if resp.clicked() {
                        app.go_to_page(*id);
                    }
                    if resp.double_clicked() {
                        app.report.renaming_page = Some((*id, name.clone()));
                    }
                    if resp.dragged()
                        && let Some(pp) = ui.input(|inp| inp.pointer.hover_pos())
                    {
                        let shift = ((pp.x - r.center().x) / 100.0).round() as i64;
                        if shift != 0 {
                            let j = (i as i64 + shift).clamp(0, pages.len() as i64 - 1) as usize;
                            if j != i {
                                drag_to = Some((i, j));
                            }
                        }
                    }
                    resp.context_menu(|ui| page_menu(app, ui, *id));
                }
                if let Some((a, b)) = drag_to {
                    app.checkpoint("Move page");
                    let p = app.project.report.pages.remove(a);
                    app.project.report.pages.insert(b.min(app.project.report.pages.len()), p);
                }
                let add = ui.add(egui::Button::new(RichText::new("+").strong().size(15.0)).frame(false)).on_hover_text("New page (right-click for worksheet / dashboard / story)");
                if add.clicked() {
                    super::new_page(app, PageKind::Report);
                }
                add.context_menu(|ui| {
                    for (k, label) in [(PageKind::Report, "Report page"), (PageKind::Worksheet, "Worksheet"), (PageKind::Dashboard, "Dashboard"), (PageKind::Tooltip, "Tooltip page")] {
                        if ui.button(label).clicked() {
                            super::new_page(app, k);
                            ui.close();
                        }
                    }
                    if ui.button("Story").clicked() {
                        super::story::new_story(app);
                        ui.close();
                    }
                });
                ui.add_space(6.0);
                for (k, icon, tip) in [(PageKind::Worksheet, "worksheet", "New worksheet"), (PageKind::Dashboard, "dashboard", "New dashboard")] {
                    let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, 4.0, t.panel_alt);
                    }
                    crate::icons::paint(ui.painter(), r.shrink(4.0), icon, t.muted);
                    if resp.on_hover_text(tip).clicked() {
                        super::new_page(app, k);
                    }
                }
                let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, 4.0, t.panel_alt);
                }
                crate::icons::paint(ui.painter(), r.shrink(4.0), "story", t.muted);
                if resp.on_hover_text("New story").clicked() {
                    super::story::new_story(app);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if app.report.reading { "Editing view" } else { "Reading view" };
                    if ui.button(label).on_hover_text("Reading view: interact like a report consumer (buttons run on click)").clicked() {
                        app.report.reading = !app.report.reading;
                    }
                    if ui.toggle_value(&mut app.report.mobile, "Mobile layout").changed() {
                        app.report.fit = true;
                    }
                    if app.rt.focus.is_some() && ui.button("Back to report").clicked() {
                        app.rt.focus = None;
                    }
                    if !app.rt.back_stack.is_empty() && ui.button("← Back").clicked() {
                        super::actions::run(app, &Action::Back);
                    }
                });
            });
        });
    });
    // Rename dialog.
    if let Some((id, mut name)) = app.report.renaming_page.clone() {
        let mut done = false;
        let mut cancel = false;
        egui::Window::new("Rename page").collapsible(false).resizable(false).show(ui.ctx(), |ui| {
            let r = ui.text_edit_singleline(&mut name);
            r.request_focus();
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                done = true;
            }
            ui.horizontal(|ui| {
                if ui.button("Rename").clicked() {
                    done = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if done {
            app.checkpoint("Rename page");
            if let Some(p) = app.project.page_mut(id) {
                p.name = name;
            }
            app.report.renaming_page = None;
        } else if cancel {
            app.report.renaming_page = None;
        } else {
            app.report.renaming_page = Some((id, name));
        }
    }
}

fn page_menu(app: &mut VizApp, ui: &mut Ui, id: u64) {
    if ui.button("Rename").clicked() {
        let n = app.project.page(id).map(|p| p.name.clone()).unwrap_or_default();
        app.report.renaming_page = Some((id, n));
        ui.close();
    }
    if ui.button("Duplicate").clicked() {
        app.page = id;
        super::duplicate_page(app);
        ui.close();
    }
    let hidden = app.project.page(id).is_some_and(|p| p.hidden);
    if ui.button(if hidden { "Unhide page" } else { "Hide page" }).clicked() {
        app.checkpoint("Hide page");
        if let Some(p) = app.project.page_mut(id) {
            p.hidden = !hidden;
        }
        ui.close();
    }
    ui.menu_button("Convert to", |ui| {
        for (k, label) in [(PageKind::Report, "Report page"), (PageKind::Worksheet, "Worksheet"), (PageKind::Dashboard, "Dashboard"), (PageKind::Tooltip, "Tooltip page"), (PageKind::Drillthrough, "Drill-through page")] {
            if ui.button(label).clicked() {
                app.checkpoint("Page type");
                if let Some(p) = app.project.page_mut(id) {
                    p.kind = k;
                    if k == PageKind::Worksheet && p.worksheet_visual.is_none() {
                        p.worksheet_visual = p.elements.iter().find(|e| e.visual().is_some()).map(|e| e.id);
                    }
                }
                ui.close();
            }
        }
    });
    if ui.button("Delete page").clicked() {
        if app.project.report.pages.len() > 1 {
            app.checkpoint("Delete page");
            app.project.report.pages.retain(|p| p.id != id);
            if app.page == id {
                app.page = app.project.report.pages.first().map(|p| p.id).unwrap_or(0);
            }
        } else {
            app.error("A report needs at least one page.");
        }
        ui.close();
    }
}

/// Format pane with nothing selected: page settings.
pub fn page_format(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let Some(mut page) = app.current_page().cloned() else { return };
    let mut changed = false;
    ui.label(RichText::new("Page settings").strong());
    widgets::section(ui, "pg-info", "Page information", true, |ui| {
        ui.horizontal(|ui| {
            ui.label("Name");
            changed |= ui.text_edit_singleline(&mut page.name).changed();
        });
        ui.horizontal(|ui| {
            ui.label("Type");
            egui::ComboBox::from_id_salt("pg-kind").selected_text(page.kind.label()).show_ui(ui, |ui| {
                for k in [PageKind::Report, PageKind::Worksheet, PageKind::Dashboard, PageKind::Tooltip, PageKind::Drillthrough] {
                    changed |= ui.selectable_value(&mut page.kind, k, k.label()).changed();
                }
            });
        });
        changed |= ui.checkbox(&mut page.hidden, "Hide page").changed();
    });
    widgets::section(ui, "pg-size", "Canvas settings", true, |ui| {
        ui.horizontal(|ui| {
            ui.label("Size");
            let presets = [("16:9", 1280.0, 720.0), ("4:3", 960.0, 720.0), ("Letter", 816.0, 1056.0), ("Tooltip", 320.0, 240.0), ("Phone", 360.0, 640.0), ("Dashboard 1366", 1366.0, 768.0), ("Dashboard 1920", 1920.0, 1080.0)];
            egui::ComboBox::from_id_salt("pg-size").selected_text(page.size_preset.clone()).show_ui(ui, |ui| {
                for (name, w, h) in presets {
                    if ui.selectable_label(page.size_preset == name, name).clicked() {
                        page.size_preset = name.into();
                        page.width = w;
                        page.height = h;
                        changed = true;
                        app.report.fit = true;
                    }
                }
                if ui.selectable_label(page.size_preset == "Custom", "Custom").clicked() {
                    page.size_preset = "Custom".into();
                    changed = true;
                }
            });
        });
        ui.horizontal(|ui| {
            ui.label("W");
            changed |= ui.add(egui::DragValue::new(&mut page.width).range(200.0..=8000.0)).changed();
            ui.label("H");
            changed |= ui.add(egui::DragValue::new(&mut page.height).range(200.0..=8000.0)).changed();
        });
        ui.horizontal(|ui| {
            ui.label("Vertical alignment");
            for a in ["Top", "Middle"] {
                changed |= ui.selectable_value(&mut page.vertical_align, a.to_string(), a).changed();
            }
        });
    });
    widgets::section(ui, "pg-bg", "Canvas background", true, |ui| {
        changed |= super::design::fill_editor(ui, "pgbg", &mut page.background);
        if ui.button("Background image…").clicked()
            && let Some(path) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg"]).pick_file()
            && let Ok(bytes) = std::fs::read(&path)
        {
            let key = format!("bg-{}", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
            app.project.report.images.insert(key.clone(), super::b64_encode(&bytes));
            page.background_image = Some(key);
            changed = true;
        }
        if page.background_image.is_some() && ui.button("Remove image").clicked() {
            page.background_image = None;
            changed = true;
        }
    });
    widgets::section(ui, "pg-wall", "Wallpaper", false, |ui| {
        changed |= super::design::fill_editor(ui, "pgwall", &mut page.wallpaper);
    });
    widgets::section(ui, "pg-dt", "Drill through", false, |ui| {
        ui.label(RichText::new("Fields that pass from the source page (drag fields from the Data pane onto the list in the data menu: Add to drill-through).").size(10.5).color(t.muted));
        let mut rm = None;
        for (i, f) in page.drillthrough.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(f.label());
                if ui.small_button("✕").clicked() {
                    rm = Some(i);
                }
            });
        }
        if let Some(i) = rm {
            page.drillthrough.remove(i);
            changed = true;
        }
        changed |= ui.checkbox(&mut page.keep_all_filters, "Keep all filters").changed();
    });
    if changed {
        app.checkpoint("Page settings");
        if let Some(p) = app.current_page_mut() {
            *p = page;
        }
        app.report.render_cache.clear();
    }
}

/// The phone layout designer for the current page.
pub fn mobile(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let Some(page) = app.current_page().cloned() else { return };
    egui::Panel::left("mobile-visuals").default_size(230.0).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::same(8))).show(ui, |ui| {
        ui.label(RichText::new("Page visuals").strong());
        ui.label(RichText::new("Click to place on the phone layout.").size(10.5).color(t.muted));
        if ui.button("Auto-generate layout").clicked() {
            auto_mobile(app);
        }
        if ui.button("Clear layout").clicked() {
            app.checkpoint("Clear mobile layout");
            if let Some(p) = app.current_page_mut() {
                p.mobile.items.clear();
            }
        }
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for e in page.elements.iter().filter(|e| e.kind != ElementKind::Group && !e.hidden) {
                let placed = page.mobile.items.iter().any(|m| m.element == e.id);
                let label = format!("{} {}", if placed { "✓" } else { "+" }, if e.name.is_empty() { e.kind_label() } else { e.name.clone() });
                if ui.selectable_label(placed, label).clicked() {
                    app.checkpoint("Mobile layout");
                    if let Some(p) = app.current_page_mut() {
                        if placed {
                            p.mobile.items.retain(|m| m.element != e.id);
                        } else {
                            let y = p.mobile.items.iter().map(|m| m.y + m.h).fold(0.0, f32::max) + 8.0;
                            let h = (e.h * 340.0 / e.w.max(1.0)).clamp(60.0, 420.0);
                            p.mobile.items.push(MobileItem { element: e.id, x: 10.0, y, w: 340.0, h });
                        }
                    }
                }
            }
        });
    });
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.canvas)).show(ui, |ui| {
        let avail = ui.available_rect_before_wrap();
        let phone_w = 360.0;
        let content_h = page.mobile.items.iter().map(|m| m.y + m.h).fold(640.0, f32::max) + 20.0;
        let zoom = ((avail.height() - 60.0) / 700.0).clamp(0.5, 1.4);
        let frame = egui::Rect::from_center_size(egui::pos2(avail.center().x, avail.top() + 20.0 + 700.0 * zoom / 2.0), vec2((phone_w + 24.0) * zoom, 700.0 * zoom));
        ui.painter().rect(frame, 36.0 * zoom, Color32::from_rgb(0x11, 0x14, 0x1B), Stroke::NONE, StrokeKind::Inside);
        let screen = frame.shrink(12.0 * zoom);
        ui.painter().rect_filled(screen, 26.0 * zoom, crate::theme::c32(app.project.report.theme.background));
        ui.painter().rect_filled(egui::Rect::from_center_size(egui::pos2(frame.center().x, frame.top() + 22.0 * zoom), vec2(90.0 * zoom, 18.0 * zoom)), 9.0 * zoom, Color32::from_rgb(0x11, 0x14, 0x1B));
        let clip = screen.shrink2(vec2(0.0, 30.0 * zoom));
        let scroll_id = ui.scope_id().with("mscroll");
        let mut scroll: f32 = ui.ctx().data_mut(|d| *d.get_temp_mut_or(scroll_id, 0.0));
        if ui.rect_contains_pointer(clip) {
            scroll = (scroll - ui.input(|i| i.smooth_scroll_delta.y) / zoom).clamp(0.0, (content_h - 600.0).max(0.0));
        }
        ui.ctx().data_mut(|d| d.insert_temp(scroll_id, scroll));
        let p = ui.painter_at(clip);
        let xf = super::canvas::PageXf { origin: clip.min - vec2(0.0, scroll * zoom), zoom };
        let items = page.mobile.items.clone();
        for m in &items {
            let Some(el) = page.element(m.element) else { continue };
            let mut e2 = el.clone();
            e2.x = m.x;
            e2.y = m.y;
            e2.w = m.w;
            e2.h = m.h;
            let r = xf.el_rect(&e2);
            if !clip.intersects(r) {
                continue;
            }
            if e2.visual().is_some() {
                let rd = super::render::element(app, page.id, &e2);
                crate::paint::paint_scene(&p, &rd.chrome, crate::paint::Xf { origin: r.min, scale: zoom });
                crate::paint::paint_scene(&p.with_clip_rect(r.intersect(clip)), &rd.content, crate::paint::Xf { origin: r.min + vec2(rd.content_rect.x, rd.content_rect.y) * zoom, scale: zoom });
            } else {
                p.rect(r, 6.0, t.panel, Stroke::new(1.0, t.border), StrokeKind::Inside);
                p.text(r.center(), egui::Align2::CENTER_CENTER, e2.kind_label(), egui::FontId::proportional(11.0), t.muted);
            }
            // Move by dragging vertically; resize from the bottom edge.
            let id = ui.scope_id().with(("mitem", m.element));
            let resp = ui.interact(r.intersect(clip), id, Sense::drag());
            if resp.dragged() {
                let d = resp.drag_delta() / zoom;
                let near_bottom = resp.interact_pointer_pos().is_some_and(|pp| (pp.y - r.bottom()).abs() < 12.0);
                if let Some(pg) = app.current_page_mut()
                    && let Some(mi) = pg.mobile.items.iter_mut().find(|x| x.element == m.element)
                {
                    if near_bottom {
                        mi.h = (mi.h + d.y).max(40.0);
                    } else {
                        mi.y = (mi.y + d.y).max(0.0);
                        mi.x = (mi.x + d.x).clamp(0.0, 360.0 - mi.w);
                    }
                }
                app.dirty = true;
            }
            if resp.hovered() {
                p.rect_stroke(r, 4.0, Stroke::new(1.5, t.accent), StrokeKind::Outside);
            }
        }
        if items.is_empty() {
            p.text(clip.center(), egui::Align2::CENTER_CENTER, "Add visuals from the left\nor Auto-generate", egui::FontId::proportional(13.0), t.muted);
        }
    });
}

pub fn auto_mobile(app: &mut VizApp) {
    app.checkpoint("Auto mobile layout");
    let Some(p) = app.current_page_mut() else { return };
    let mut els: Vec<&Element> = p.elements.iter().filter(|e| e.kind != ElementKind::Group && !e.hidden && e.w > 40.0 && e.h > 20.0).collect();
    els.sort_by(|a, b| ((a.y / 40.0).round(), a.x).partial_cmp(&((b.y / 40.0).round(), b.x)).unwrap_or(std::cmp::Ordering::Equal));
    let mut y = 8.0;
    let mut items = Vec::new();
    for e in els {
        let card = e.visual().is_some_and(|v| matches!(v.kind, VisualKind::Card | VisualKind::Kpi));
        let h = if card { 110.0 } else if e.visual().is_some() { 260.0 } else { (e.h * 340.0 / e.w.max(1.0)).clamp(30.0, 200.0) };
        items.push(MobileItem { element: e.id, x: 10.0, y, w: 340.0, h });
        y += h + 8.0;
    }
    p.mobile.items = items;
    p.mobile.width = 360.0;
}
