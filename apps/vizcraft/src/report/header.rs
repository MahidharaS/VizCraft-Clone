//! The visual header (drill up/down, expand, focus mode, open as sheet ↗, more options) and
//! page/bookmark navigators.

use egui::{Align2, Color32, FontId, Painter, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use vizcraft_core::doc::*;

use super::render::Rendered;
use crate::app::VizApp;
use crate::theme::Tokens;

struct Icon {
    id: &'static str,
    glyph: &'static str,
    tip: &'static str,
}

pub fn paint(app: &mut VizApp, ui: &Ui, p: &Painter, el: &Element, r: Rect, t: &Tokens, rd: &Rendered) {
    let Some(v) = el.visual() else { return };
    let mut icons: Vec<Icon> = Vec::new();
    let levels = rd.data.as_ref().map(|d| d.drill_levels).unwrap_or(0);
    if levels > 1 && rd.props.bool("header.drill", true) {
        icons.push(Icon { id: "up", glyph: "↑", tip: "Drill up" });
        icons.push(Icon { id: "mode", glyph: if v.drill.drill_mode { "⇣" } else { "↓" }, tip: if v.drill.drill_mode { "Drill mode is on: click a data point to drill down" } else { "Turn on drill down (click a data point to drill)" } });
        icons.push(Icon { id: "next", glyph: "⇊", tip: "Go to the next level in the hierarchy" });
        icons.push(Icon { id: "expand", glyph: "⋔", tip: "Expand all down one level in the hierarchy" });
    }
    icons.push(Icon { id: "filter", glyph: "▽", tip: "Filters and slicers affecting this visual" });
    if rd.props.bool("header.focus", true) {
        icons.push(Icon { id: "focus", glyph: if app.rt.focus == Some(el.id) { "⤡" } else { "⤢" }, tip: if app.rt.focus == Some(el.id) { "Back to report" } else { "Focus mode" } });
    }
    if rd.props.bool("header.openSheet", true) {
        icons.push(Icon { id: "sheet", glyph: "↗", tip: "Open as a sheet: a new worksheet page just for this visual" });
    }
    icons.push(Icon { id: "more", glyph: "⋯", tip: "More options" });
    let size = 20.0;
    let total = icons.len() as f32 * size;
    let start = pos2((r.right() - total - 6.0).max(r.left() + 2.0), r.top() + 3.0);
    let bar = Rect::from_min_size(start - vec2(2.0, 0.0), vec2(total + 4.0, size));
    p.rect(bar, 4.0, t.panel, Stroke::new(1.0, t.border), StrokeKind::Inside);
    for (i, ic) in icons.iter().enumerate() {
        let ir = Rect::from_min_size(start + vec2(i as f32 * size, 0.0), vec2(size, size));
        let id = ui.scope_id().with(("hdr", el.id, ic.id));
        let resp = ui.interact(ir, id, Sense::click());
        let active = (ic.id == "mode" && v.drill.drill_mode) || (ic.id == "focus" && app.rt.focus == Some(el.id));
        if resp.hovered() || active {
            p.rect_filled(ir.shrink(1.0), 3.0, t.accent_soft);
        }
        p.text(ir.center(), Align2::CENTER_CENTER, ic.glyph, FontId::proportional(13.0), if active { t.accent } else { t.text });
        if resp.hovered() {
            app.report.header_consumed = true;
        }
        let resp = resp.on_hover_text(ic.tip);
        if resp.clicked() {
            app.report.header_consumed = true;
            match ic.id {
                "up" => super::actions::drill(app, el.id, "up"),
                "mode" => super::actions::drill(app, el.id, "mode"),
                "next" => super::actions::drill(app, el.id, "next"),
                "expand" => super::actions::drill(app, el.id, "expand"),
                "focus" => {
                    app.rt.focus = if app.rt.focus == Some(el.id) { None } else { Some(el.id) };
                }
                "sheet" => super::open_as_sheet(app, el.id),
                "filter" => {
                    let d = rd.cx.description.clone();
                    app.toast(if d.is_empty() { "No filters affect this visual.".to_string() } else { format!("Filters: {}", d.join("; ")) });
                }
                _ => {
                    app.selected = vec![el.id];
                    app.report.more_menu = Some(el.id);
                }
            }
        }
    }
    // The "more options" menu as a popup at the header.
    if app.report.more_menu == Some(el.id) {
        let mut open = true;
        egui::Area::new(ui.scope_id().with(("more", el.id))).fixed_pos(pos2(r.right() - 190.0, r.top() + 2.0)).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(180.0);
                super::menus::element_menu(app, ui);
                if ui.button("Close").clicked() {
                    open = false;
                }
            });
        });
        if !open || ui.input(|i| i.key_pressed(egui::Key::Escape)) || (ui.input(|i| i.pointer.any_click()) && !app.report.header_consumed) {
            app.report.more_menu = None;
        }
    }
}

/// Page / bookmark navigator buttons drawn inside a button element.
pub fn paint_navigator(app: &VizApp, p: &Painter, el: &Element, r: Rect, nav: &str, zoom: f32, t: &Tokens) {
    let items = navigator_items(app, nav);
    if items.is_empty() {
        return;
    }
    let n = items.len() as f32;
    let horizontal = el.w >= el.h;
    for (i, (label, active)) in items.iter().enumerate() {
        let br = if horizontal {
            Rect::from_min_size(r.min + vec2(i as f32 * r.width() / n, 0.0), vec2(r.width() / n - 4.0 * zoom, r.height()))
        } else {
            Rect::from_min_size(r.min + vec2(0.0, i as f32 * r.height() / n), vec2(r.width(), r.height() / n - 4.0 * zoom))
        };
        p.rect(br, 6.0 * zoom, if *active { t.accent } else { Color32::WHITE }, Stroke::new(1.0, t.accent), StrokeKind::Inside);
        p.text(br.center(), Align2::CENTER_CENTER, label, FontId::proportional(12.0 * zoom), if *active { Color32::WHITE } else { t.accent });
    }
}

fn navigator_items(app: &VizApp, nav: &str) -> Vec<(String, bool)> {
    if nav == "bookmarks" {
        app.project.report.bookmarks.iter().map(|b| (b.name.clone(), app.report.last_bookmark == Some(b.id))).collect()
    } else {
        app.project.report.pages.iter().filter(|p| !p.hidden && matches!(p.kind, PageKind::Report | PageKind::Dashboard)).map(|p| (p.name.clone(), p.id == app.page)).collect()
    }
}

pub fn navigator_click(app: &mut VizApp, el: &Element, _xf: super::canvas::PageXf, px: f32, py: f32) {
    let Some(nav) = (match &el.kind {
        ElementKind::Button(b) => b.navigator.clone(),
        _ => None,
    }) else {
        return;
    };
    let n = navigator_items(app, &nav).len();
    if n == 0 {
        return;
    }
    let idx = if el.w >= el.h { ((px - el.x) / el.w * n as f32) as usize } else { ((py - el.y) / el.h * n as f32) as usize };
    if nav == "bookmarks" {
        if let Some(b) = app.project.report.bookmarks.get(idx).cloned() {
            super::bookmarks::apply(app, &b);
        }
    } else {
        let pages: Vec<u64> = app.project.report.pages.iter().filter(|p| !p.hidden && matches!(p.kind, PageKind::Report | PageKind::Dashboard)).map(|p| p.id).collect();
        if let Some(id) = pages.get(idx) {
            app.go_to_page(*id);
        }
    }
}
