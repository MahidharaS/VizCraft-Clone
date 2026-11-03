//! Small building blocks of the Power Query UI: ribbon tools and groups, dialog windows,
//! combo boxes, OK/Cancel rows.

use egui::{Align2, Color32, FontId, Response, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::theme::Tokens;

/// The caption of a ribbon tool on one or two lines (long captions wrap at the middle space).
fn caption_lines(label: &str) -> Vec<String> {
    if let Some((a, b)) = label.split_once('\n') {
        return vec![a.to_string(), b.to_string()];
    }
    if label.chars().count() <= 9 || !label.contains(' ') {
        return vec![label.to_string()];
    }
    let mid = label.len() / 2;
    let split = label.char_indices().filter(|(_, c)| *c == ' ').min_by_key(|(i, _)| (*i as i64 - mid as i64).abs()).map(|(i, _)| i);
    match split {
        Some(i) => vec![label.get(..i).unwrap_or_default().to_string(), label.get(i + 1..).unwrap_or_default().to_string()],
        None => vec![label.to_string()],
    }
}

fn text_w(ui: &Ui, s: &str, size: f32) -> f32 {
    ui.ctx().fonts_mut(|f| f.layout_no_wrap(s.to_string(), FontId::proportional(size), Color32::BLACK).size().x)
}

/// A large ribbon tool: an icon over a one- or two-line caption, with an optional dropdown
/// arrow. `active` draws it pressed (a toggle that is on).
pub fn tool(ui: &mut Ui, t: &Tokens, icon: &str, label: &str, tip: &str, dropdown: bool, active: bool) -> Response {
    let lines = caption_lines(label);
    let tw = lines.iter().map(|l| text_w(ui, l, 11.0)).fold(0.0, f32::max) + if dropdown { 10.0 } else { 0.0 };
    let w = (tw + 12.0).max(44.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 66.0), Sense::click());
    let p = ui.painter();
    if active {
        p.rect(rect.shrink(1.0), 3.0, t.accent_soft, Stroke::new(1.0, t.accent.gamma_multiply(0.6)), StrokeKind::Inside);
    } else if resp.hovered() {
        p.rect_filled(rect.shrink(1.0), 3.0, t.accent_soft);
    }
    let icon_rect = egui::Rect::from_center_size(pos2(rect.center().x, rect.top() + 17.0), vec2(24.0, 24.0));
    super::icons::paint(p, icon_rect, icon, t.text, t.dark);
    let mut y = rect.top() + 35.0;
    let n = lines.len();
    for (i, l) in lines.iter().enumerate() {
        let last = i + 1 == n;
        let x = rect.center().x - if dropdown && last { 5.0 } else { 0.0 };
        p.text(pos2(x, y), Align2::CENTER_TOP, l, FontId::proportional(11.0), t.text);
        if dropdown && last {
            let lw = text_w(ui, l, 11.0);
            caret(p, pos2(x + lw / 2.0 + 6.0, y + 7.0), 3.5, t.muted, 'd');
        }
        y += 13.0;
    }
    if dropdown && n == 0 {
        caret(p, pos2(rect.center().x, y + 6.0), 3.5, t.muted, 'd');
    }
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// A small triangle pointing down (`d`), right (`r`), up (`u`) or left (`l`).
pub fn caret(p: &egui::Painter, c: egui::Pos2, s: f32, color: Color32, dir: char) {
    let pts = match dir {
        'r' => vec![c + vec2(-s * 0.6, -s), c + vec2(s * 0.7, 0.0), c + vec2(-s * 0.6, s)],
        'u' => vec![c + vec2(-s, s * 0.6), c + vec2(s, s * 0.6), c + vec2(0.0, -s * 0.7)],
        'l' => vec![c + vec2(s * 0.6, -s), c + vec2(-s * 0.7, 0.0), c + vec2(s * 0.6, s)],
        _ => vec![c + vec2(-s, -s * 0.6), c + vec2(s, -s * 0.6), c + vec2(0.0, s * 0.7)],
    };
    p.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
}

/// An × drawn with two lines.
pub fn cross(p: &egui::Painter, c: egui::Pos2, s: f32, color: Color32) {
    p.line_segment([c + vec2(-s, -s), c + vec2(s, s)], Stroke::new(1.4, color));
    p.line_segment([c + vec2(-s, s), c + vec2(s, -s)], Stroke::new(1.4, color));
}

/// A check mark.
pub fn check(p: &egui::Painter, c: egui::Pos2, s: f32, color: Color32) {
    p.add(egui::Shape::line(vec![c + vec2(-s, 0.0), c + vec2(-s * 0.3, s * 0.7), c + vec2(s, -s * 0.7)], Stroke::new(1.8, color)));
}

/// A small square button that paints a glyph ('x' cross, 'v' check, 'u' up, 'd' down).
pub fn glyph_button(ui: &mut Ui, t: &Tokens, glyph: char, color: Color32, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, t.accent_soft);
    }
    let c = rect.center();
    match glyph {
        'x' => cross(ui.painter(), c, 4.0, color),
        'v' => check(ui.painter(), c, 5.0, color),
        g => caret(ui.painter(), c, 4.0, color, g),
    }
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// A small ribbon tool: a 16px icon and a caption on one line.
pub fn small_tool(ui: &mut Ui, t: &Tokens, icon: &str, label: &str, tip: &str, dropdown: bool) -> Response {
    let w = text_w(ui, label, 11.0) + 28.0 + if dropdown { 10.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 21.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, t.accent_soft);
    }
    super::icons::paint(ui.painter(), egui::Rect::from_min_size(rect.min + vec2(3.0, 2.5), vec2(16.0, 16.0)), icon, t.text, t.dark);
    ui.painter().text(rect.min + vec2(23.0, 10.5), Align2::LEFT_CENTER, label, FontId::proportional(11.0), t.text);
    if dropdown {
        caret(ui.painter(), rect.right_center() - vec2(6.0, 0.0), 3.5, t.muted, 'd');
    }
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// A ribbon group: tools in a row, the caption under them and a divider on the right.
pub fn group(ui: &mut Ui, t: &Tokens, caption: &str, add: impl FnOnce(&mut Ui)) {
    let r = ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = vec2(1.0, 0.0);
        ui.horizontal_top(|ui| {
            ui.set_min_height(68.0);
            add(ui);
        });
        let w = ui.min_rect().width();
        let (cr, _) = ui.allocate_exact_size(vec2(w.max(40.0), 14.0), Sense::hover());
        ui.painter().text(cr.center(), Align2::CENTER_CENTER, caption, FontId::proportional(10.0), t.muted);
    });
    let rect = r.response.rect;
    ui.add_space(4.0);
    let x = rect.right() + 3.0;
    ui.painter().line_segment([pos2(x, rect.top() + 4.0), pos2(x, rect.bottom() - 2.0)], Stroke::new(1.0, t.border));
    ui.add_space(5.0);
}

/// Stacks small tools in a column.
pub fn stack(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.add_space(1.0);
        add(ui);
    });
}

/// A dialog window, centred, not collapsible.
pub fn window(title: &str) -> egui::Window<'_> {
    egui::Window::new(RichText::new(title).strong().size(14.0))
        .id(egui::Id::unique(("pq-dialog", title)))
        .collapsible(false)
        .resizable(true)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(pos2(0.0, 0.0))
}

/// Positions a window in the middle of the screen with a default size.
pub fn fit<'a>(w: egui::Window<'a>, ctx: &egui::Context, size: egui::Vec2) -> egui::Window<'a> {
    let screen = ctx.content_rect();
    let size = vec2(size.x.min(screen.width() - 40.0), size.y.min(screen.height() - 80.0));
    w.default_pos(screen.center()).default_width(size.x).min_width(size.x.min(360.0)).max_height(screen.height() - 60.0)
}

/// A combo of text options bound to a string.
pub fn combo(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, sel: &mut String, options: &[String], width: f32) -> Response {
    let shown = if sel.is_empty() { "(choose)".to_string() } else { sel.clone() };
    egui::ComboBox::from_id_salt(id)
        .width(width)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            for o in options {
                ui.selectable_value(sel, o.clone(), o.as_str());
            }
        })
        .response
}

/// A combo bound to an index into `options`.
pub fn index_combo(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, sel: &mut usize, options: &[&str], width: f32) -> Response {
    let shown = options.get(*sel).copied().unwrap_or_default();
    egui::ComboBox::from_id_salt(id)
        .width(width)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            for (i, o) in options.iter().enumerate() {
                ui.selectable_value(sel, i, *o);
            }
        })
        .response
}

/// An error message line (or nothing).
pub fn error_line(ui: &mut Ui, t: &Tokens, err: &Option<String>) {
    if let Some(e) = err {
        ui.add_space(4.0);
        egui::Frame::new().fill(t.danger.gamma_multiply(0.12)).corner_radius(4.0).inner_margin(egui::Margin::symmetric(8, 5)).show(ui, |ui| {
            ui.label(RichText::new(e).color(t.danger));
        });
    }
}

/// A note in a soft panel (information or warning).
pub fn note(ui: &mut Ui, t: &Tokens, text: &str, warn: bool) {
    let c = if warn { Color32::from_rgb(0xB4, 0x53, 0x09) } else { t.accent };
    egui::Frame::new().fill(c.gamma_multiply(0.10)).stroke(Stroke::new(1.0, c.gamma_multiply(0.35))).corner_radius(4.0).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
        ui.label(RichText::new(text).size(11.5).color(if warn { c } else { t.text }));
    });
}

/// An accent-filled button.
pub fn primary(ui: &mut Ui, t: &Tokens, label: &str) -> Response {
    ui.add(egui::Button::new(RichText::new(label).color(Color32::WHITE).strong()).fill(t.accent).min_size(vec2(84.0, 26.0)))
}

/// A plain button with a minimum size that matches [`primary`].
pub fn secondary(ui: &mut Ui, label: &str) -> Response {
    ui.add(egui::Button::new(label).min_size(vec2(76.0, 26.0)))
}

/// OK / Cancel on the right; returns (ok, cancel). Enter is OK (outside multi-line editors),
/// Escape is Cancel.
pub fn ok_cancel(ui: &mut Ui, t: &Tokens, ok_label: &str) -> (bool, bool) {
    ui.add_space(8.0);
    let mut ok = false;
    let mut cancel = false;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        cancel = secondary(ui, "Cancel").clicked();
        ok = primary(ui, t, ok_label).clicked();
    });
    let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
    (ok, cancel || esc)
}

/// A section caption in dialogs and panes (small, muted, upper-case).
pub fn caption(ui: &mut Ui, t: &Tokens, s: &str) {
    ui.label(RichText::new(s.to_uppercase()).size(10.0).color(t.muted).strong());
}

/// A label + widget row in a two-column form grid.
pub fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.label(label);
    add(ui);
    ui.end_row();
}
