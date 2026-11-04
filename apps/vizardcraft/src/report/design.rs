//! Figma-style design properties: fills (solid, linear, radial gradients), strokes, corner
//! radius, shadows, rich text runs (each run bindable to data), buttons and images.

use egui::{RichText, Ui};
use vizardcraft_core::color::Color;
use vizardcraft_core::doc::*;

use crate::app::VizApp;
use crate::theme::Tokens;
use crate::widgets;

fn edit<F: FnOnce(&mut Element)>(app: &mut VizApp, id: u64, label: &str, f: F) {
    app.checkpoint(label);
    if let Some(e) = app.project.element_mut(id) {
        f(e);
    }
    app.report.render_cache.clear();
}

pub fn fill_editor(ui: &mut Ui, id_salt: impl std::hash::Hash + Copy, fill: &mut Fill) -> bool {
    let mut changed = false;
    let kinds = ["None", "Solid", "Linear", "Radial"];
    let cur = match fill {
        Fill::None => 0,
        Fill::Solid(_) => 1,
        Fill::Linear { .. } => 2,
        Fill::Radial { .. } => 3,
    };
    let mut sel = cur;
    ui.horizontal(|ui| {
        for (i, k) in kinds.iter().enumerate() {
            ui.selectable_value(&mut sel, i, *k);
        }
    });
    if sel != cur {
        let base = fill.color().unwrap_or(Color::rgb(0x2B, 0x6C, 0xE0));
        *fill = match sel {
            0 => Fill::None,
            1 => Fill::Solid(base),
            2 => Fill::Linear { angle: 90.0, stops: vec![(0.0, base), (1.0, base.shade(0.5))] },
            _ => Fill::Radial { stops: vec![(0.0, base.shade(0.4)), (1.0, base)] },
        };
        changed = true;
    }
    match fill {
        Fill::Solid(c) => {
            ui.horizontal(|ui| {
                ui.label("Color");
                changed |= widgets::color_edit(ui, c).changed();
                let mut a = c.a as f32 / 255.0 * 100.0;
                if ui.add(egui::Slider::new(&mut a, 0.0..=100.0).suffix("%")).changed() {
                    c.a = (a / 100.0 * 255.0) as u8;
                    changed = true;
                }
            });
        }
        Fill::Linear { angle, stops } => {
            ui.horizontal(|ui| {
                ui.label("Angle");
                changed |= ui.add(egui::DragValue::new(angle).range(0.0..=360.0).suffix("°")).changed();
            });
            changed |= stops_editor(ui, id_salt, stops);
        }
        Fill::Radial { stops } => {
            changed |= stops_editor(ui, id_salt, stops);
        }
        Fill::None => {}
    }
    changed
}

fn stops_editor(ui: &mut Ui, id_salt: impl std::hash::Hash + Copy, stops: &mut Vec<(f32, Color)>) -> bool {
    let mut changed = false;
    let mut remove = None;
    for (i, (pos, c)) in stops.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            changed |= widgets::color_edit(ui, c).changed();
            changed |= ui.add(egui::Slider::new(pos, 0.0..=1.0).text("stop")).changed();
            if ui.small_button("✕").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove
        && stops.len() > 2
    {
        stops.remove(i);
        changed = true;
    }
    if ui.small_button("+ stop").clicked() {
        stops.push((1.0, Color::WHITE));
        changed = true;
    }
    let _ = id_salt;
    changed
}

pub fn stroke_editor(ui: &mut Ui, s: &mut Stroke) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Stroke");
        changed |= ui.add(egui::DragValue::new(&mut s.width).range(0.0..=40.0).speed(0.25)).changed();
        changed |= widgets::color_edit(ui, &mut s.color).changed();
        egui::ComboBox::from_id_salt(ui.scope_id().with("dash")).width(64.0).selected_text(s.dash.as_str()).show_ui(ui, |ui| {
            for d in ["solid", "dash", "dot"] {
                changed |= ui.selectable_value(&mut s.dash, d.to_string(), d).changed();
            }
        });
    });
    changed
}

pub fn shape_props(app: &mut VizApp, ui: &mut Ui, t: &Tokens, id: u64) {
    let Some(ElementKind::Shape(spec)) = app.project.element(id).map(|(_, e)| e.kind.clone()) else { return };
    widgets::section(ui, "shape-design", "Shape (design)", true, |ui| {
        let mut s = spec.clone();
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Shape");
            egui::ComboBox::from_id_salt(("shapekind", id)).selected_text(format!("{:?}", s.shape)).show_ui(ui, |ui| {
                for k in ShapeKind::ALL {
                    changed |= ui.selectable_value(&mut s.shape, k, format!("{k:?}")).changed();
                }
            });
        });
        ui.label(RichText::new("Fill").strong());
        changed |= fill_editor(ui, ("sf", id), &mut s.fill);
        changed |= stroke_editor(ui, &mut s.stroke);
        ui.label(RichText::new("Text").strong());
        let mut txt = s.text.raw_text();
        if ui.add(egui::TextEdit::multiline(&mut txt).desired_rows(2).hint_text("Label (supports {[Measure]})")).changed() {
            s.text = RichText2::plain(&txt);
            changed = true;
        }
        if s.shape == ShapeKind::Frame {
            ui.horizontal(|ui| {
                ui.label("Auto layout");
                egui::ComboBox::from_id_salt(("al", id)).selected_text(s.auto_layout.as_str()).show_ui(ui, |ui| {
                    for m in ["none", "horizontal", "vertical"] {
                        changed |= ui.selectable_value(&mut s.auto_layout, m.to_string(), m).changed();
                    }
                });
                changed |= ui.add(egui::DragValue::new(&mut s.gap).prefix("gap ")).changed();
            });
            if ui.button("Apply auto layout to elements inside").clicked() {
                auto_layout(app, id, &s);
            }
        }
        if changed {
            edit(app, id, "Shape", |e| e.kind = ElementKind::Shape(s));
        }
    });
    let _ = t;
}

type RichText2 = vizardcraft_core::doc::RichText;

/// Frame auto layout: stacks the elements inside the frame horizontally or vertically.
fn auto_layout(app: &mut VizApp, frame: u64, s: &ShapeSpec) {
    let Some((_, f)) = app.project.element(frame).map(|(p, e)| (p, e.clone())) else { return };
    app.checkpoint("Auto layout");
    let Some(pg) = app.current_page_mut() else { return };
    let mut inside: Vec<(u64, f32, f32)> = pg.elements.iter().filter(|e| e.id != frame && e.x >= f.x && e.y >= f.y && e.x + e.w <= f.x + f.w && e.y + e.h <= f.y + f.h).map(|e| (e.id, e.x, e.y)).collect();
    let horizontal = s.auto_layout == "horizontal";
    inside.sort_by(|a, b| if horizontal { a.1.partial_cmp(&b.1) } else { a.2.partial_cmp(&b.2) }.unwrap_or(std::cmp::Ordering::Equal));
    let pad = f.style.padding.max(8.0);
    let mut cur = if horizontal { f.x + pad } else { f.y + pad };
    for (id, _, _) in inside {
        if let Some(e) = pg.element_mut(id) {
            if horizontal {
                e.x = cur;
                e.y = f.y + pad;
                cur += e.w + s.gap;
            } else {
                e.y = cur;
                e.x = f.x + pad;
                cur += e.h + s.gap;
            }
        }
    }
}

pub fn text_props(app: &mut VizApp, ui: &mut Ui, t: &Tokens, id: u64) {
    let Some(ElementKind::Text(ts)) = app.project.element(id).map(|(_, e)| e.kind.clone()) else { return };
    widgets::section(ui, "text-runs", "Text (rich)", true, |ui| {
        let mut ts2 = ts.clone();
        let mut changed = false;
        let mut fx_for: Option<usize> = None;
        ui.horizontal(|ui| {
            ui.label("Align");
            for a in ["left", "center", "right"] {
                changed |= ui.selectable_value(&mut ts2.text.align, a.to_string(), a).changed();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Vertical");
            for a in ["top", "middle", "bottom"] {
                changed |= ui.selectable_value(&mut ts2.text.valign, a.to_string(), a).changed();
            }
        });
        let mut remove = None;
        for (i, run) in ts2.text.runs.iter_mut().enumerate() {
            egui::Frame::new().fill(t.panel_alt).corner_radius(4.0).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
                changed |= ui.add(egui::TextEdit::multiline(&mut run.text).desired_rows(1).desired_width(f32::INFINITY)).changed();
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::DragValue::new(&mut run.size).range(4.0..=200.0).suffix("pt")).changed();
                    changed |= ui.toggle_value(&mut run.bold, RichText::new("B").strong()).changed();
                    changed |= ui.toggle_value(&mut run.italic, RichText::new("I").italics()).changed();
                    changed |= ui.toggle_value(&mut run.underline, RichText::new("U").underline()).changed();
                    let mut c = match &run.color {
                        Some(Prop::Color(c)) => *c,
                        _ => app.project.report.theme.foreground,
                    };
                    if matches!(run.color, Some(Prop::Fx(_))) {
                        if ui.button(RichText::new("ƒx color").color(t.accent)).clicked() {
                            fx_for = Some(i);
                        }
                    } else {
                        if widgets::color_edit(ui, &mut c).changed() {
                            run.color = Some(Prop::Color(c));
                            changed = true;
                        }
                        if ui.small_button("ƒx").on_hover_text("Colour this run by data").clicked() {
                            fx_for = Some(i);
                        }
                    }
                    if ui.small_button("✕").clicked() {
                        remove = Some(i);
                    }
                });
            });
        }
        if let Some(i) = remove {
            ts2.text.runs.remove(i);
            changed = true;
        }
        if ui.button("+ Add run").on_hover_text("A run is a piece of text with its own style").clicked() {
            let size = ts2.text.runs.last().map(|r| r.size).unwrap_or(14.0);
            ts2.text.runs.push(TextRun { text: " {[Total Sales]:$#,0}".into(), size, bold: true, ..TextRun::default() });
            changed = true;
        }
        changed |= ui.checkbox(&mut ts2.smart_narrative, "Smart narrative (auto summary)").changed();
        if changed {
            edit(app, id, "Text", |e| e.kind = ElementKind::Text(ts2));
            app.report.text_cache.clear();
        }
        if let Some(i) = fx_for {
            let cur = ts.text.runs.get(i).and_then(|r| r.color.clone()).and_then(|p| if let Prop::Fx(f) = p { Some(f) } else { None });
            super::fx_dialog::open_owned(app, id, format!("run.{i}.color"), vizardcraft_core::format::PropKind::Color, cur);
        }
    });
}

pub fn button_props(app: &mut VizApp, ui: &mut Ui, t: &Tokens, id: u64) {
    let Some(ElementKind::Button(b)) = app.project.element(id).map(|(_, e)| e.kind.clone()) else { return };
    widgets::section(ui, "btn-action", "Button action", true, |ui| {
        let mut b2 = b.clone();
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Label");
            let mut s = b2.text.raw_text();
            if ui.text_edit_singleline(&mut s).changed() {
                b2.text = RichText2::plain(&s);
                changed = true;
            }
        });
        ui.label(RichText::new("Fill").strong());
        changed |= fill_editor(ui, ("bf", id), &mut b2.fill);
        ui.label(RichText::new("Hover fill").strong());
        changed |= fill_editor(ui, ("bh", id), &mut b2.hover_fill);
        let kinds = ["None", "Back", "Bookmark", "Page navigation", "Drill through", "Web URL", "Q&A", "Clear all slicers", "Set parameter"];
        let cur = match &b2.action {
            None => 0,
            Some(Action::Back) => 1,
            Some(Action::Bookmark { .. }) => 2,
            Some(Action::PageNavigation { .. }) => 3,
            Some(Action::Drillthrough { .. }) => 4,
            Some(Action::Url { .. }) => 5,
            Some(Action::QandA) => 6,
            Some(Action::ClearAllSlicers) | Some(Action::ApplyAllSlicers) => 7,
            Some(Action::SetParameter { .. }) => 8,
        };
        let mut sel = cur;
        ui.horizontal(|ui| {
            ui.label("Action");
            egui::ComboBox::from_id_salt(("act", id)).selected_text(*kinds.get(cur).unwrap_or(&"None")).show_ui(ui, |ui| {
                for (i, k) in kinds.iter().enumerate() {
                    ui.selectable_value(&mut sel, i, *k);
                }
            });
        });
        let first_page = app.project.report.pages.first().map(|p| p.id).unwrap_or(0);
        let first_bm = app.project.report.bookmarks.first().map(|b| b.id).unwrap_or(0);
        if sel != cur {
            b2.action = match sel {
                1 => Some(Action::Back),
                2 => Some(Action::Bookmark { id: first_bm }),
                3 => Some(Action::PageNavigation { page: first_page }),
                4 => Some(Action::Drillthrough { page: first_page }),
                5 => Some(Action::Url { url: "https://".into() }),
                6 => Some(Action::QandA),
                7 => Some(Action::ClearAllSlicers),
                8 => Some(Action::SetParameter { name: app.project.model.parameters.first().map(|p| p.name.clone()).unwrap_or_default(), value: vizardcraft_core::scalar::Scalar::Num(0.0) }),
                _ => None,
            };
            changed = true;
        }
        match &mut b2.action {
            Some(Action::PageNavigation { page }) | Some(Action::Drillthrough { page }) => {
                let pages: Vec<(u64, String)> = app.project.report.pages.iter().map(|p| (p.id, p.name.clone())).collect();
                egui::ComboBox::from_id_salt(("actpage", id)).selected_text(pages.iter().find(|p| p.0 == *page).map(|p| p.1.clone()).unwrap_or_default()).show_ui(ui, |ui| {
                    for (pid, name) in &pages {
                        changed |= ui.selectable_value(page, *pid, name).changed();
                    }
                });
            }
            Some(Action::Bookmark { id: bid }) => {
                let bms: Vec<(u64, String)> = app.project.report.bookmarks.iter().map(|b| (b.id, b.name.clone())).collect();
                egui::ComboBox::from_id_salt(("actbm", id)).selected_text(bms.iter().find(|b| b.0 == *bid).map(|b| b.1.clone()).unwrap_or_else(|| "(add a bookmark first)".into())).show_ui(ui, |ui| {
                    for (i, name) in &bms {
                        changed |= ui.selectable_value(bid, *i, name).changed();
                    }
                });
            }
            Some(Action::Url { url }) => {
                changed |= ui.text_edit_singleline(url).changed();
            }
            Some(Action::SetParameter { name, value }) => {
                changed |= ui.text_edit_singleline(name).changed();
                let mut s = value.text();
                if ui.text_edit_singleline(&mut s).changed() {
                    *value = vizardcraft_core::sheet::parse_input(&s);
                    changed = true;
                }
            }
            _ => {}
        }
        ui.label(RichText::new("Ctrl+click runs the action while editing (like Power BI).").size(10.5).color(t.muted));
        if changed {
            edit(app, id, "Button", |e| e.kind = ElementKind::Button(b2));
        }
    });
}

pub fn image_props(app: &mut VizApp, ui: &mut Ui, t: &Tokens, id: u64) {
    let Some(ElementKind::Image(img)) = app.project.element(id).map(|(_, e)| e.kind.clone()) else { return };
    widgets::section(ui, "img", "Image", true, |ui| {
        let mut i2 = img.clone();
        let mut changed = false;
        if ui.button("Choose image…").clicked()
            && let Some(path) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg"]).pick_file()
            && let Ok(bytes) = std::fs::read(&path)
        {
            let key = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "image".into());
            app.project.report.images.insert(key.clone(), super::b64_encode(&bytes));
            i2.source = key;
            changed = true;
        }
        ui.horizontal(|ui| {
            ui.label("Scaling");
            for s in ["fit", "fill", "normal"] {
                changed |= ui.selectable_value(&mut i2.scaling, s.to_string(), s).changed();
            }
        });
        ui.label(RichText::new(format!("Source: {}", i2.source)).size(10.5).color(t.muted));
        if changed {
            edit(app, id, "Image", |e| e.kind = ElementKind::Image(i2));
        }
    });
}

/// Container style shared by every element: background, border, radius, shadow.
pub fn box_style(app: &mut VizApp, ui: &mut Ui, t: &Tokens, id: u64) {
    let Some(style) = app.project.element(id).map(|(_, e)| e.style.clone()) else { return };
    widgets::section(ui, "boxstyle", "Container (fill, border, radius, shadow)", false, |ui| {
        let mut s = style.clone();
        let mut changed = false;
        let mut has_fill = s.fill.is_some();
        if ui.checkbox(&mut has_fill, "Custom background").changed() {
            s.fill = if has_fill { Some(Fill::Solid(Color::WHITE)) } else { None };
            changed = true;
        }
        if let Some(f) = s.fill.as_mut() {
            changed |= fill_editor(ui, ("box", id), f);
        }
        changed |= stroke_editor(ui, &mut s.stroke);
        ui.horizontal(|ui| {
            ui.label("Radius");
            let mut uniform = s.radius.first().copied().unwrap_or(-1.0);
            if ui.add(egui::DragValue::new(&mut uniform).range(-1.0..=200.0)).on_hover_text("-1 = theme default").changed() {
                s.radius = [uniform; 4];
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            changed |= ui.checkbox(&mut s.shadow.enabled, "Shadow").changed();
            changed |= widgets::color_edit(ui, &mut s.shadow.color).changed();
            changed |= ui.add(egui::DragValue::new(&mut s.shadow.blur).prefix("blur ")).changed();
            changed |= ui.add(egui::DragValue::new(&mut s.shadow.y).prefix("y ")).changed();
        });
        if changed {
            edit(app, id, "Container style", |e| e.style = s);
        }
    });
    let _ = t;
}
