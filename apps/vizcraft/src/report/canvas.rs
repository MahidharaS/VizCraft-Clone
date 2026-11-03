//! The report canvas: paints the page and its elements, and handles selection, moving,
//! resizing, rotating, smart guides, marquee selection, data-point clicks (cross-filter,
//! drill, slicers, buttons), visual headers and tooltips.

use egui::{Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use vizcraft_core::doc::*;
use vizcraft_core::scalar::Scalar;

use super::{Drag, DragKind};
use crate::app::VizApp;
use crate::paint::{Xf, paint_scene};
use crate::theme::{Tokens, c32};

pub const HANDLE: f32 = 7.0;

/// Screen transform of the page.
#[derive(Clone, Copy)]
pub struct PageXf {
    pub origin: Pos2,
    pub zoom: f32,
}

impl PageXf {
    pub fn to_screen(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.origin.x + x * self.zoom, self.origin.y + y * self.zoom)
    }
    pub fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.to_screen(x, y), vec2(w * self.zoom, h * self.zoom))
    }
    pub fn el_rect(&self, e: &Element) -> Rect {
        self.rect(e.x, e.y, e.w, e.h)
    }
    pub fn to_page(&self, p: Pos2) -> (f32, f32) {
        ((p.x - self.origin.x) / self.zoom, (p.y - self.origin.y) / self.zoom)
    }
}

/// Draws the page and handles interaction.
pub fn show(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let Some(page) = app.current_page().cloned() else {
        ui.centered_and_justified(|ui| ui.label("No page. Insert → New page."));
        return;
    };
    let avail = ui.available_rect_before_wrap();
    let (resp_rect, bg_resp) = ui.allocate_exact_size(avail.size(), Sense::click_and_drag());
    let painter = ui.painter_at(resp_rect);
    painter.rect_filled(resp_rect, 0.0, t.canvas);
    // Zoom / fit.
    if app.report.fit {
        app.report.zoom = ((resp_rect.width() - 48.0) / page.width).min((resp_rect.height() - 48.0) / page.height).clamp(0.1, 4.0);
        app.report.pan = Vec2::ZERO;
    }
    if app.report.fit_width {
        app.report.zoom = ((resp_rect.width() - 48.0) / page.width).clamp(0.1, 4.0);
        app.report.pan = Vec2::ZERO;
        app.report.fit_width = false;
    }
    // Ctrl+scroll zooms; scroll pans.
    if bg_resp.hovered() || ui.rect_contains_pointer(resp_rect) {
        let (scroll, zoom_delta, ctrl) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers.command));
        if zoom_delta != 1.0 {
            app.report.zoom = (app.report.zoom * zoom_delta).clamp(0.1, 4.0);
            app.report.fit = false;
        } else if !ctrl && scroll != Vec2::ZERO {
            app.report.pan += scroll;
            app.report.fit = false;
        }
    }
    let zoom = app.report.zoom;
    let size = vec2(page.width * zoom, page.height * zoom);
    let origin = pos2((resp_rect.center().x - size.x / 2.0).max(resp_rect.left() + 24.0), (resp_rect.center().y - size.y / 2.0).max(resp_rect.top() + 24.0)) + app.report.pan;
    let xf = PageXf { origin, zoom };
    let page_rect = Rect::from_min_size(origin, size);
    // Page shadow + background.
    for i in 1..5 {
        painter.rect_filled(page_rect.expand(i as f32 * 1.5).translate(vec2(0.0, 2.0)), 3.0, Color32::from_black_alpha(10));
    }
    paint_fill(&painter, page_rect, &page.wallpaper);
    let bg = match &page.background {
        Fill::None => Fill::Solid(vizcraft_core::color::Color::WHITE),
        f => f.clone(),
    };
    let bg = if matches!(bg, Fill::Solid(c) if c == vizcraft_core::color::Color::WHITE) && app.project.report.theme.background != vizcraft_core::color::Color::rgb(0xF4, 0xF5, 0xF7) && matches!(page.background, Fill::None) {
        Fill::Solid(app.project.report.theme.background)
    } else {
        bg
    };
    super::shapes::fill(&painter, &super::shapes::rounded(page_rect, 0.0), &bg, page_rect, 1.0);
    if let Some(img) = page.background_image.clone()
        && let Some(b64) = app.project.report.images.get(&img).cloned()
        && let Some(tex) = app.texture(ui.ctx(), &format!("img:{img}"), &b64)
    {
        painter.image(tex.id(), page_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    if app.project.report.settings.show_gridlines {
        let g = app.project.report.settings.grid_size.max(4.0) * 2.0;
        let mut x = 0.0;
        while x < page.width {
            painter.line_segment([xf.to_screen(x, 0.0), xf.to_screen(x, page.height)], Stroke::new(1.0, Color32::from_black_alpha(14)));
            x += g;
        }
        let mut y = 0.0;
        while y < page.height {
            painter.line_segment([xf.to_screen(0.0, y), xf.to_screen(page.width, y)], Stroke::new(1.0, Color32::from_black_alpha(14)));
            y += g;
        }
    }
    let page_painter = ui.painter_at(page_rect.intersect(resp_rect));
    // Elements in z-order.
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let mut hover_hit: Option<(u64, vizcraft_core::scene::Hit)> = None;
    let focus = app.rt.focus;
    let spotlight = app.rt.spotlight;
    let elements: Vec<Element> = page.elements.clone();
    for el in &elements {
        if el.hidden || el.kind == ElementKind::Group {
            continue;
        }
        if focus.is_some_and(|f| f != el.id) {
            continue;
        }
        let r = if focus == Some(el.id) {
            // Focus mode: the visual fills the page.
            let mut e2 = el.clone();
            e2.x = 16.0;
            e2.y = 16.0;
            e2.w = page.width - 32.0;
            e2.h = page.height - 32.0;
            paint_element(app, ui, &page_painter, &page, &e2, xf, t, pointer, &mut hover_hit);
            continue;
        } else {
            xf.el_rect(el)
        };
        let _ = r;
        paint_element(app, ui, &page_painter, &page, el, xf, t, pointer, &mut hover_hit);
        if spotlight.is_some_and(|s| s != el.id) {
            page_painter.rect_filled(xf.el_rect(el).expand(2.0), 0.0, Color32::from_white_alpha(170));
        }
    }
    // Selection chrome (not in reading view).
    let sel: Vec<Element> = if app.report.reading { vec![] } else { elements.iter().filter(|e| app.selected.contains(&e.id)).cloned().collect() };
    for el in &sel {
        let r = xf.el_rect(el);
        painter.rect_stroke(r, 2.0, Stroke::new(1.5, t.selection), StrokeKind::Outside);
        if sel.len() == 1 && !el.locked {
            for (_, hp) in handles(r) {
                painter.rect(Rect::from_center_size(hp, vec2(HANDLE, HANDLE)), 1.0, Color32::WHITE, Stroke::new(1.2, t.selection), StrokeKind::Middle);
            }
            if !matches!(el.kind, ElementKind::Visual(_)) {
                let rp = pos2(r.center().x, r.top() - 18.0);
                painter.line_segment([rp, pos2(r.center().x, r.top())], Stroke::new(1.0, t.selection));
                painter.circle(rp, 4.5, Color32::WHITE, Stroke::new(1.2, t.selection));
            }
        }
        if el.locked {
            painter.text(r.right_top() + vec2(-4.0, 4.0), Align2::RIGHT_TOP, "locked", FontId::proportional(9.0), t.muted);
        }
    }
    if sel.len() > 1 {
        let mut u = xf.el_rect(&sel[0]);
        for e in &sel {
            u = u.union(xf.el_rect(e));
        }
        painter.rect_stroke(u.expand(3.0), 2.0, Stroke::new(1.0, t.selection.gamma_multiply(0.6)), StrokeKind::Outside);
    }
    // Groups: outline of the selected group.
    for el in elements.iter().filter(|e| e.kind == ElementKind::Group && app.selected.contains(&e.id)) {
        let kids: Vec<&Element> = elements.iter().filter(|k| k.parent == Some(el.id)).collect();
        if let Some(first) = kids.first() {
            let mut u = xf.el_rect(first);
            for k in &kids {
                u = u.union(xf.el_rect(k));
            }
            painter.rect_stroke(u.expand(4.0), 2.0, Stroke::new(1.2, t.selection), StrokeKind::Outside);
        }
    }
    // Smart guides.
    for (vertical, pos) in &app.report.guides {
        if *vertical {
            painter.line_segment([xf.to_screen(*pos, 0.0), xf.to_screen(*pos, page.height)], Stroke::new(1.0, Color32::from_rgb(0xF4, 0x3F, 0x5E)));
        } else {
            painter.line_segment([xf.to_screen(0.0, *pos), xf.to_screen(page.width, *pos)], Stroke::new(1.0, Color32::from_rgb(0xF4, 0x3F, 0x5E)));
        }
    }
    if let Some(Drag { kind: DragKind::Marquee, start, .. }) = &app.report.drag
        && let Some(pp) = pointer
    {
        let a = xf.to_screen(start.0, start.1);
        let r = Rect::from_two_pos(a, pp);
        painter.rect(r, 0.0, t.accent.gamma_multiply(0.08), Stroke::new(1.0, t.accent), StrokeKind::Inside);
    }
    // Edit interactions overlay.
    if app.report.edit_interactions
        && let [src] = app.selected.as_slice()
    {
        let src = *src;
        for el in elements.iter().filter(|e| e.id != src && e.visual().is_some() && !e.hidden) {
            let r = xf.el_rect(el);
            let cur = el.interactions.get(&src).copied().unwrap_or_default();
            let opts = [(Interaction::Filter, "filter"), (Interaction::Highlight, "highlight"), (Interaction::None, "none")];
            for (i, (it, label)) in opts.iter().enumerate() {
                let br = Rect::from_min_size(r.right_top() + vec2(-((3 - i) as f32) * 58.0 - 4.0, 4.0), vec2(56.0, 18.0));
                let on = cur == *it || (cur == Interaction::Default && *it == Interaction::Highlight && app.project.report.settings.cross_highlight);
                painter.rect(br, 9.0, if on { t.accent } else { Color32::WHITE }, Stroke::new(1.0, t.accent), StrokeKind::Inside);
                painter.text(br.center(), Align2::CENTER_CENTER, *label, FontId::proportional(10.0), if on { Color32::WHITE } else { t.accent });
                let id = ui.scope_id().with(("int", el.id, i));
                let r2 = ui.interact(br, id, Sense::click());
                if r2.clicked() {
                    app.checkpoint("Edit interactions");
                    if let Some(e) = app.project.element_mut(el.id) {
                        e.interactions.insert(src, *it);
                    }
                    app.report.render_cache.clear();
                }
            }
        }
    }
    // Interaction.
    interact(app, ui, &page, &elements, xf, &bg_resp, resp_rect, t);
    // Tooltip.
    if let Some((eid, hit)) = hover_hit
        && app.report.drag.is_none()
    {
        tooltip(app, ui, &page, eid, &hit, pointer.unwrap_or_default(), t);
    }
}

fn paint_fill(p: &egui::Painter, r: Rect, f: &Fill) {
    if !matches!(f, Fill::None) {
        super::shapes::fill(p, &super::shapes::rounded(r, 0.0), f, r, 1.0);
    }
}

pub fn handles(r: Rect) -> [(u8, Pos2); 8] {
    [
        (0, r.left_top()),
        (1, pos2(r.center().x, r.top())),
        (2, r.right_top()),
        (3, pos2(r.right(), r.center().y)),
        (4, r.right_bottom()),
        (5, pos2(r.center().x, r.bottom())),
        (6, r.left_bottom()),
        (7, pos2(r.left(), r.center().y)),
    ]
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_element(app: &mut VizApp, ui: &Ui, p: &egui::Painter, page: &Page, el: &Element, xf: PageXf, t: &Tokens, pointer: Option<Pos2>, hover: &mut Option<(u64, vizcraft_core::scene::Hit)>) {
    let r = xf.el_rect(el);
    if !p.clip_rect().intersects(r.expand(30.0)) {
        return;
    }
    let opacity = el.style.opacity.clamp(0.0, 1.0);
    match &el.kind {
        ElementKind::Visual(_) => {
            let rd = super::render::element(app, page.id, el);
            let cp = p.with_clip_rect(r.expand(12.0).intersect(p.clip_rect()));
            paint_scene(&cp, &rd.chrome, Xf { origin: r.min, scale: xf.zoom });
            let content_origin = r.min + vec2(rd.content_rect.x, rd.content_rect.y) * xf.zoom;
            let content_rect = Rect::from_min_size(content_origin, vec2(rd.content_rect.w, rd.content_rect.h) * xf.zoom);
            let scroll = app.report.scroll.get(&el.id).copied().unwrap_or(0.0);
            let cp2 = p.with_clip_rect(content_rect.intersect(p.clip_rect()));
            paint_scene(&cp2, &rd.content, Xf { origin: content_origin - vec2(0.0, scroll * xf.zoom), scale: xf.zoom });
            // Script visual image.
            for it in &rd.content.items {
                if let vizcraft_core::scene::Shape::Image { rect, png_base64 } = &it.shape {
                    let key = format!("script:{}:{}", el.id, png_base64.len());
                    if let Some(tex) = app.texture(ui.ctx(), &key, png_base64) {
                        let ir = Rect::from_min_size(content_origin + vec2(rect.x, rect.y) * xf.zoom, vec2(rect.w, rect.h) * xf.zoom);
                        let sz = tex.size_vec2();
                        let s = (ir.width() / sz.x).min(ir.height() / sz.y);
                        let fit = Rect::from_center_size(ir.center(), sz * s);
                        cp2.image(tex.id(), fit, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                }
            }
            if let Some(pp) = pointer
                && content_rect.contains(pp)
            {
                let local = ((pp.x - content_origin.x) / xf.zoom, (pp.y - content_origin.y) / xf.zoom + scroll);
                if let Some(h) = rd.content.hit_at(local.0, local.1) {
                    *hover = Some((el.id, h.clone()));
                }
            }
            // Visual header (hover or selected).
            let hovered = pointer.is_some_and(|pp| r.expand(4.0).contains(pp));
            if (hovered || app.selected.contains(&el.id)) && rd.props.bool("header.show", true) {
                super::header::paint(app, ui, p, el, r, t, &rd);
            }
        }
        ElementKind::Shape(s) => {
            let fill = el.props.get("shape.fill").map(|pr| super::render::resolve(app, page, el, pr)).and_then(|pr| pr.as_color()).map(Fill::Solid).unwrap_or_else(|| s.fill.clone());
            let radius = el.props.get("shape.radius").and_then(|p| p.as_num()).filter(|v| *v >= 0.0).map(|v| v as f32).unwrap_or(el.style.radius.first().copied().unwrap_or(-1.0));
            let mut pts = super::shapes::outline(s.shape, r, if radius >= 0.0 { radius * xf.zoom } else { -1.0 }, &s.points);
            let rot = el.rotation + el.props.get("shape.rotation").and_then(|p| p.as_num()).unwrap_or(0.0) as f32;
            super::shapes::rotate(&mut pts, r.center(), rot);
            if el.style.shadow.enabled {
                let sh = el.style.shadow.clone();
                for i in 0..4 {
                    let g = sh.blur * xf.zoom * (i as f32 + 1.0) / 4.0;
                    let off = vec2(sh.x, sh.y) * xf.zoom;
                    let mut sp = super::shapes::outline(s.shape, r.expand(g / 2.0).translate(off), radius.max(0.0) * xf.zoom + g, &s.points);
                    super::shapes::rotate(&mut sp, r.center() + off, rot);
                    super::shapes::fill(p, &sp, &Fill::Solid(sh.color.with_alpha((sh.color.a as f32 / (3.0 + i as f32)) as u8)), r, opacity);
                }
            }
            if s.shape != ShapeKind::Line {
                super::shapes::fill(p, &pts, &fill, r, opacity);
            }
            let mut st = s.stroke.clone();
            if let Some(c) = el.props.get("shape.stroke").and_then(|pr| super::render::resolve(app, page, el, pr).as_color()) {
                st.color = c;
                st.width = st.width.max(1.0);
            }
            if let Some(w) = el.props.get("shape.strokeWidth").and_then(|p| p.as_num()).filter(|w| *w >= 0.0) {
                st.width = w as f32;
            }
            if s.shape == ShapeKind::Line && st.width <= 0.0 {
                st.width = 2.0;
                st.color = fill.color().unwrap_or(vizcraft_core::color::Color::BLACK);
            }
            super::shapes::stroke_outline(p, &pts, &st, s.shape != ShapeKind::Line && (s.shape != ShapeKind::Path || s.closed), xf.zoom, opacity);
            let text = el.props.get("shape.text").map(|pr| super::render::resolve(app, page, el, pr).as_text()).filter(|s| !s.is_empty()).unwrap_or_else(|| s.text.raw_text());
            if !text.is_empty() {
                let text = super::render::template(app, page, el, &text);
                let size = el.props.get("shape.textSize").and_then(|p| p.as_num()).unwrap_or(s.text.runs.first().map(|r| r.size as f64).unwrap_or(14.0)) as f32;
                let col = el.props.get("shape.textColor").and_then(|pr| super::render::resolve(app, page, el, pr).as_color()).map(c32).unwrap_or_else(|| fill.color().map(|c| c32(c.contrast_text())).unwrap_or(t.text));
                let g = p.layout(text, FontId::proportional(size * xf.zoom), col, r.width() - 12.0 * xf.zoom);
                p.galley(r.center() - g.size() / 2.0, g, col);
            }
        }
        ElementKind::Text(ts) => {
            paint_chrome_simple(app, p, page, el, r, xf);
            super::text::paint_rich(app, ui, p, page, el, &ts.text, r.shrink(el.style.padding * xf.zoom), xf.zoom, opacity, t);
        }
        ElementKind::Image(img) => {
            paint_chrome_simple(app, p, page, el, r, xf);
            let src = el.props.get("image.url").map(|pr| super::render::resolve(app, page, el, pr).as_text()).filter(|s| !s.is_empty()).unwrap_or(img.source.clone());
            let b64 = app.project.report.images.get(&src).cloned().or_else(|| src.starts_with("data:").then(|| src.clone()));
            match b64.and_then(|b| app.texture(ui.ctx(), &format!("img:{src}"), &b)) {
                Some(tex) => {
                    let sz = tex.size_vec2();
                    let inner = r.shrink(el.style.padding.max(0.0) * xf.zoom * 0.0);
                    let target = match img.scaling.as_str() {
                        "fill" | "Fill" => inner,
                        "normal" | "Normal" => Rect::from_min_size(inner.min, sz * xf.zoom),
                        _ => {
                            let s = (inner.width() / sz.x).min(inner.height() / sz.y);
                            Rect::from_center_size(inner.center(), sz * s)
                        }
                    };
                    p.with_clip_rect(r).image(tex.id(), target, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(opacity));
                }
                None => {
                    p.rect(r, 4.0, t.panel_alt, Stroke::new(1.0, t.border), StrokeKind::Inside);
                    crate::icons::paint(p, Rect::from_center_size(r.center(), vec2(40.0, 40.0)), "image", t.muted);
                }
            }
        }
        ElementKind::Button(b) => {
            let hovered = pointer.is_some_and(|pp| r.contains(pp));
            let fill = el.props.get(if hovered { "button.hoverFill" } else { "button.fill" }).and_then(|pr| super::render::resolve(app, page, el, pr).as_color()).map(Fill::Solid).unwrap_or_else(|| if hovered { b.hover_fill.clone() } else { b.fill.clone() });
            let rad = el.props.get("button.radius").and_then(|p| p.as_num()).unwrap_or(6.0) as f32 * xf.zoom;
            super::shapes::fill(p, &super::shapes::rounded(r, rad), &fill, r, opacity);
            let label = el.props.get("button.text").map(|pr| super::render::resolve(app, page, el, pr).as_text()).filter(|s| !s.is_empty()).unwrap_or_else(|| b.text.raw_text());
            let label = super::render::template(app, page, el, &label);
            let col = el.props.get("button.textColor").and_then(|pr| super::render::resolve(app, page, el, pr).as_color()).map(c32).unwrap_or_else(|| fill.color().map(|c| c32(c.contrast_text())).unwrap_or(Color32::WHITE));
            let size = el.props.get("button.textSize").and_then(|p| p.as_num()).unwrap_or(12.0) as f32 * xf.zoom;
            let icon = match b.icon.as_str() {
                "Back" => "← ",
                "Bookmark" => "",
                "Information" => "i  ",
                "Help" => "?  ",
                "Arrow" => "→ ",
                "Reset" => "↺ ",
                _ => "",
            };
            if let Some(nav) = &b.navigator {
                super::header::paint_navigator(app, p, el, r, nav, xf.zoom, t);
            } else {
                p.text(r.center(), Align2::CENTER_CENTER, format!("{icon}{label}"), FontId::proportional(size), col);
            }
            if hovered && b.action.is_some() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
        }
        ElementKind::Group => {}
    }
}

fn paint_chrome_simple(app: &mut VizApp, p: &egui::Painter, page: &Page, el: &Element, r: Rect, xf: PageXf) {
    let has_bg = el.style.fill.is_some() || el.props.get("background.show").and_then(|b| b.as_bool()).unwrap_or(false) || el.style.stroke.width > 0.0 || el.style.shadow.enabled;
    if !has_bg {
        return;
    }
    let rd = super::render::element(app, page.id, el);
    paint_scene(p, &rd.chrome, Xf { origin: r.min, scale: xf.zoom });
}

/// The topmost element under a page point.
pub fn element_at(elements: &[Element], x: f32, y: f32) -> Option<&Element> {
    elements.iter().rev().find(|e| !e.hidden && e.kind != ElementKind::Group && e.contains(x, y))
}

/// The selectable target of an element: its top-level group, if grouped.
fn top_group(elements: &[Element], id: u64) -> u64 {
    let mut cur = id;
    for _ in 0..20 {
        match elements.iter().find(|e| e.id == cur).and_then(|e| e.parent) {
            Some(p) => cur = p,
            None => break,
        }
    }
    cur
}

fn group_members(elements: &[Element], id: u64) -> Vec<u64> {
    let mut out = vec![id];
    let mut i = 0;
    while i < out.len() {
        let cur = out[i];
        out.extend(elements.iter().filter(|e| e.parent == Some(cur)).map(|e| e.id));
        i += 1;
        if out.len() > 500 {
            break;
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn interact(app: &mut VizApp, ui: &mut Ui, page: &Page, elements: &[Element], xf: PageXf, bg: &egui::Response, area: Rect, t: &Tokens) {
    let _ = t;
    let Some(pos) = ui.input(|i| i.pointer.interact_pos()) else { return };
    let (px, py) = xf.to_page(pos);
    let mods = ui.input(|i| i.modifiers);
    let locked_all = app.project.report.settings.lock_objects;
    // Cursor feedback over handles.
    if app.selected.len() == 1
        && let Some(el) = elements.iter().find(|e| app.selected.contains(&e.id))
        && !el.locked
        && !locked_all
    {
        let r = xf.el_rect(el);
        for (h, hp) in handles(r) {
            if Rect::from_center_size(hp, vec2(HANDLE + 4.0, HANDLE + 4.0)).contains(pos) {
                ui.ctx().set_cursor_icon(match h {
                    0 | 4 => CursorIcon::ResizeNwSe,
                    2 | 6 => CursorIcon::ResizeNeSw,
                    1 | 5 => CursorIcon::ResizeVertical,
                    _ => CursorIcon::ResizeHorizontal,
                });
            }
        }
    }
    if !area.contains(pos) && app.report.drag.is_none() {
        return;
    }
    // Header clicks are handled by header widgets; skip canvas handling when they consumed it.
    if app.report.header_consumed {
        app.report.header_consumed = false;
        return;
    }
    if bg.drag_started() {
        start_drag(app, elements, xf, pos, px, py, mods);
    }
    if bg.dragged() && let Some(drag) = app.report.drag.clone() {
        update_drag(app, elements, page, &drag, px, py, mods);
    }
    if bg.drag_stopped() {
        if let Some(Drag { kind: DragKind::Marquee, start, .. }) = app.report.drag.clone() {
            let r = Rect::from_two_pos(pos2(start.0, start.1), pos2(px, py));
            let hits: Vec<u64> = elements.iter().filter(|e| !e.hidden && e.kind != ElementKind::Group && r.intersects(Rect::from_min_size(pos2(e.x, e.y), vec2(e.w, e.h)))).map(|e| top_group(elements, e.id)).collect();
            if !mods.shift && !mods.command {
                app.selected.clear();
            }
            for h in hits {
                if !app.selected.contains(&h) {
                    app.selected.push(h);
                }
            }
        }
        app.report.drag = None;
        app.report.guides.clear();
    }
    if bg.clicked() || bg.secondary_clicked() {
        click(app, ui, page, elements, xf, px, py, mods, bg.secondary_clicked());
    }
    if bg.double_clicked()
        && let Some(el) = element_at(elements, px, py)
    {
        match &el.kind {
            ElementKind::Text(_) => {
                app.report.editing_text = Some(el.id);
                app.report.text_buffer = el.kind_text();
            }
            ElementKind::Visual(_) if page.kind != PageKind::Worksheet => {
                super::open_as_sheet(app, el.id);
            }
            _ => {}
        }
    }
    // Context menu.
    bg.context_menu(|ui| super::menus::element_menu(app, ui));
}

trait KindText {
    fn kind_text(&self) -> String;
}
impl KindText for Element {
    fn kind_text(&self) -> String {
        match &self.kind {
            ElementKind::Text(t) => t.text.raw_text(),
            _ => String::new(),
        }
    }
}

fn start_drag(app: &mut VizApp, elements: &[Element], xf: PageXf, pos: Pos2, px: f32, py: f32, mods: egui::Modifiers) {
    if app.report.reading {
        return;
    }
    let locked_all = app.project.report.settings.lock_objects;
    // Resize / rotate handles of the single selection.
    if app.selected.len() == 1
        && let Some(el) = elements.iter().find(|e| app.selected.contains(&e.id))
        && !el.locked
        && !locked_all
    {
        let r = xf.el_rect(el);
        let rp = pos2(r.center().x, r.top() - 18.0);
        if !matches!(el.kind, ElementKind::Visual(_)) && rp.distance(pos) < 8.0 {
            app.checkpoint("Rotate");
            app.report.drag = Some(Drag { kind: DragKind::Rotate, start: (px, py), originals: vec![(el.id, el.x, el.y, el.w, el.h, el.rotation)] });
            return;
        }
        for (h, hp) in handles(r) {
            if Rect::from_center_size(hp, vec2(HANDLE + 4.0, HANDLE + 4.0)).contains(pos) {
                app.checkpoint("Resize");
                app.report.drag = Some(Drag { kind: DragKind::Resize(h), start: (px, py), originals: vec![(el.id, el.x, el.y, el.w, el.h, el.rotation)] });
                return;
            }
        }
    }
    match element_at(elements, px, py) {
        Some(el) => {
            let target = top_group(elements, el.id);
            if !app.selected.contains(&target) {
                if !mods.shift && !mods.command {
                    app.selected.clear();
                }
                app.selected.push(target);
            }
            if locked_all || elements.iter().any(|e| app.selected.contains(&e.id) && e.locked) {
                return;
            }
            // Dragging inside a slicer's "between" range is handled by the visual.
            app.checkpoint("Move");
            let mut ids: Vec<u64> = Vec::new();
            for s in &app.selected {
                for m in group_members(elements, *s) {
                    if !ids.contains(&m) {
                        ids.push(m);
                    }
                }
            }
            let originals = elements.iter().filter(|e| ids.contains(&e.id)).map(|e| (e.id, e.x, e.y, e.w, e.h, e.rotation)).collect();
            app.report.drag = Some(Drag { kind: DragKind::Move, start: (px, py), originals });
        }
        None => {
            app.report.drag = Some(Drag { kind: DragKind::Marquee, start: (px, py), originals: vec![] });
        }
    }
}

fn snap(v: f32, grid: f32, on: bool) -> f32 {
    if on && grid > 0.0 { (v / grid).round() * grid } else { v }
}

fn update_drag(app: &mut VizApp, elements: &[Element], page: &Page, drag: &Drag, px: f32, py: f32, mods: egui::Modifiers) {
    let (dx, dy) = (px - drag.start.0, py - drag.start.1);
    let grid = app.project.report.settings.grid_size;
    let snap_on = app.project.report.settings.snap_to_grid && !mods.alt;
    app.report.guides.clear();
    match drag.kind {
        DragKind::Move => {
            let (mut dx, mut dy) = (dx, dy);
            if mods.shift {
                if dx.abs() > dy.abs() {
                    dy = 0.0;
                } else {
                    dx = 0.0;
                }
            }
            // Bounding box of the moved elements.
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for (_, x, y, w, h, _) in &drag.originals {
                x0 = x0.min(*x);
                y0 = y0.min(*y);
                x1 = x1.max(x + w);
                y1 = y1.max(y + h);
            }
            let mut nx = x0 + dx;
            let mut ny = y0 + dy;
            let (bw, bh) = (x1 - x0, y1 - y0);
            // Smart guides against other elements and the page.
            let moving: Vec<u64> = drag.originals.iter().map(|o| o.0).collect();
            let mut xs = vec![0.0, page.width / 2.0, page.width];
            let mut ys = vec![0.0, page.height / 2.0, page.height];
            for e in elements.iter().filter(|e| !moving.contains(&e.id) && !e.hidden && e.kind != ElementKind::Group) {
                xs.extend([e.x, e.x + e.w / 2.0, e.x + e.w]);
                ys.extend([e.y, e.y + e.h / 2.0, e.y + e.h]);
            }
            let tol = 5.0 / app.report.zoom;
            let mut snapped_x = false;
            for cand in [0.0, 0.5, 1.0] {
                let edge = nx + bw * cand;
                if let Some(g) = xs.iter().find(|g| (edge - **g).abs() < tol) {
                    nx = g - bw * cand;
                    app.report.guides.push((true, *g));
                    snapped_x = true;
                    break;
                }
            }
            let mut snapped_y = false;
            for cand in [0.0, 0.5, 1.0] {
                let edge = ny + bh * cand;
                if let Some(g) = ys.iter().find(|g| (edge - **g).abs() < tol) {
                    ny = g - bh * cand;
                    app.report.guides.push((false, *g));
                    snapped_y = true;
                    break;
                }
            }
            if !snapped_x {
                nx = snap(nx, grid, snap_on);
            }
            if !snapped_y {
                ny = snap(ny, grid, snap_on);
            }
            let (ddx, ddy) = (nx - x0, ny - y0);
            if let Some(pg) = app.current_page_mut() {
                for (id, x, y, ..) in &drag.originals {
                    if let Some(e) = pg.element_mut(*id) {
                        e.x = x + ddx;
                        e.y = y + ddy;
                    }
                }
                // Keep groups' boxes in sync.
                sync_groups(pg);
            }
        }
        DragKind::Resize(h) => {
            let Some((id, x, y, w, hh, _)) = drag.originals.first().copied() else { return };
            let (mut nx, mut ny, mut nw, mut nh) = (x, y, w, hh);
            if matches!(h, 0 | 6 | 7) {
                nx = snap(x + dx, grid, snap_on);
                nw = (x + w - nx).max(8.0);
            }
            if matches!(h, 2 | 3 | 4) {
                nw = (snap(x + w + dx, grid, snap_on) - x).max(8.0);
            }
            if matches!(h, 0 | 1 | 2) {
                ny = snap(y + dy, grid, snap_on);
                nh = (y + hh - ny).max(8.0);
            }
            if matches!(h, 4 | 5 | 6) {
                nh = (snap(y + hh + dy, grid, snap_on) - y).max(8.0);
            }
            if mods.shift && w > 0.0 && hh > 0.0 {
                // Keep aspect ratio.
                let ratio = w / hh;
                if matches!(h, 1 | 5) {
                    nw = nh * ratio;
                } else {
                    nh = nw / ratio;
                }
            }
            if let Some(pg) = app.current_page_mut()
                && let Some(e) = pg.element_mut(id)
            {
                e.x = nx;
                e.y = ny;
                e.w = nw;
                e.h = nh;
            }
        }
        DragKind::Rotate => {
            let Some((id, x, y, w, h, _)) = drag.originals.first().copied() else { return };
            let c = (x + w / 2.0, y + h / 2.0);
            let mut a = (px - c.0).atan2(-(py - c.1)).to_degrees();
            if mods.shift {
                a = (a / 15.0).round() * 15.0;
            }
            if let Some(pg) = app.current_page_mut()
                && let Some(e) = pg.element_mut(id)
            {
                e.rotation = a;
            }
        }
        DragKind::Marquee => {}
    }
    app.dirty = true;
}

/// Groups take the bounding box of their children.
pub fn sync_groups(pg: &mut Page) {
    let groups: Vec<u64> = pg.elements.iter().filter(|e| e.kind == ElementKind::Group).map(|e| e.id).collect();
    for g in groups {
        let kids: Vec<(f32, f32, f32, f32)> = pg.elements.iter().filter(|e| e.parent == Some(g)).map(|e| (e.x, e.y, e.x + e.w, e.y + e.h)).collect();
        if kids.is_empty() {
            continue;
        }
        let x0 = kids.iter().map(|k| k.0).fold(f32::MAX, f32::min);
        let y0 = kids.iter().map(|k| k.1).fold(f32::MAX, f32::min);
        let x1 = kids.iter().map(|k| k.2).fold(f32::MIN, f32::max);
        let y1 = kids.iter().map(|k| k.3).fold(f32::MIN, f32::max);
        if let Some(e) = pg.element_mut(g) {
            e.x = x0;
            e.y = y0;
            e.w = x1 - x0;
            e.h = y1 - y0;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn click(app: &mut VizApp, ui: &Ui, page: &Page, elements: &[Element], xf: PageXf, px: f32, py: f32, mods: egui::Modifiers, secondary: bool) {
    let Some(el) = element_at(elements, px, py).cloned() else {
        if !secondary {
            app.selected.clear();
            app.rt.selection = None;
            app.report.editing_text = None;
            app.report.render_cache.clear();
        }
        return;
    };
    let target = top_group(elements, el.id);
    if secondary {
        if !app.selected.contains(&target) {
            app.selected = vec![target];
        }
        return;
    }
    if mods.shift || (mods.command && !matches!(el.kind, ElementKind::Visual(_))) {
        if let Some(i) = app.selected.iter().position(|s| *s == target) {
            app.selected.remove(i);
        } else {
            app.selected.push(target);
        }
        return;
    }
    // A click inside a group selects the child the second time.
    if app.selected == vec![target] && target != el.id {
        app.selected = vec![el.id];
    } else if !app.selected.contains(&target) || app.selected.len() > 1 {
        app.selected = vec![target];
    }
    // Buttons / images / shapes with actions run them (Ctrl+click while editing, like Power BI,
    // or a plain click in reading mode).
    let action = match &el.kind {
        ElementKind::Button(b) => b.action.clone(),
        ElementKind::Image(i) => i.action.clone(),
        _ => None,
    };
    if let Some(a) = action
        && (mods.command || app.report.reading)
    {
        super::actions::run(app, &a);
        return;
    }
    if let ElementKind::Button(b) = &el.kind
        && b.navigator.is_some()
    {
        super::header::navigator_click(app, &el, xf, px, py);
        return;
    }
    // Data points.
    if let ElementKind::Visual(v) = &el.kind {
        let rd = super::render::element(app, page.id, &el);
        let scroll = app.report.scroll.get(&el.id).copied().unwrap_or(0.0);
        let lx = px - el.x - rd.content_rect.x;
        let ly = py - el.y - rd.content_rect.y + scroll;
        if let Some(action) = rd.content.action_at(lx, ly).map(String::from) {
            super::actions::visual_action(app, el.id, &action, mods);
            return;
        }
        if let Some(hit) = rd.content.hit_at(lx, ly).cloned() {
            if v.drill.drill_mode && !hit.point.is_empty() {
                super::actions::drill_down_on(app, el.id, &hit.point);
                return;
            }
            vizcraft_core::state::toggle_selection(&mut app.rt, el.id, hit.point.clone(), mods.command);
            // Tableau dashboard actions.
            super::actions::dashboard_actions(app, page, el.id, &hit.point);
            app.report.render_cache.clear();
            return;
        }
        if app.rt.selection.as_ref().is_some_and(|s| s.source == el.id) {
            app.rt.selection = None;
            app.report.render_cache.clear();
        }
    }
    let _ = ui;
}

fn tooltip(app: &mut VizApp, ui: &Ui, page: &Page, eid: u64, hit: &vizcraft_core::scene::Hit, pos: Pos2, t: &Tokens) {
    let Some(el) = page.element(eid).cloned() else { return };
    let tooltip_page = el.tooltip_page.and_then(|id| app.project.page(id).cloned());
    egui::Area::new(ui.scope_id().with(("tip", eid))).fixed_pos(pos + vec2(14.0, 14.0)).order(egui::Order::Tooltip).interactable(false).show(ui.ctx(), |ui| {
        egui::Frame::new().fill(t.panel).stroke(Stroke::new(1.0, t.border)).corner_radius(6.0).inner_margin(egui::Margin::same(8)).shadow(egui::Shadow { offset: [0, 3], blur: 12, spread: 0, color: Color32::from_black_alpha(40) }).show(ui, |ui| {
            if let Some(tp) = tooltip_page {
                super::tooltip_page(app, ui, &tp, &hit.point);
            } else {
                egui::Grid::new("tipgrid").num_columns(2).spacing([12.0, 2.0]).show(ui, |ui| {
                    for (k, v) in &hit.tooltip {
                        ui.label(egui::RichText::new(k).color(t.muted).size(11.0));
                        ui.label(egui::RichText::new(v).strong().size(11.5));
                        ui.end_row();
                    }
                });
                if el.visual().is_some_and(|v| v.drill.drill_mode) {
                    ui.label(egui::RichText::new("Click to drill down").size(10.0).color(t.accent));
                }
            }
        });
    });
    let _: Option<Scalar> = None;
}

/// Paints a page without interaction (story points, tooltip pages, thumbnails).
pub fn paint_page_static(app: &mut VizApp, ui: &Ui, p: &egui::Painter, page: &Page, xf: PageXf, t: &Tokens) {
    let r = Rect::from_min_size(xf.origin, vec2(page.width * xf.zoom, page.height * xf.zoom));
    let bg = match &page.background {
        Fill::None => Fill::Solid(vizcraft_core::color::Color::WHITE),
        f => f.clone(),
    };
    super::shapes::fill(p, &super::shapes::rounded(r, 0.0), &bg, r, 1.0);
    let mut hover = None;
    for el in page.elements.iter().filter(|e| !e.hidden && e.kind != ElementKind::Group) {
        paint_element(app, ui, p, page, el, xf, t, None, &mut hover);
    }
}
