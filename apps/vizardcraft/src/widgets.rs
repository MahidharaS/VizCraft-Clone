//! Shared widgets: ribbon buttons and groups, pane headers, field chips.

use egui::{Color32, CornerRadius, Response, Sense, Stroke, StrokeKind, Ui, Vec2, vec2};

use crate::theme::Tokens;

/// A large ribbon button: icon over a caption.
pub fn big_button(ui: &mut Ui, t: &Tokens, icon: &str, label: &str, tip: &str) -> Response {
    let w = (crate::widgets::text_w(ui, label) + 12.0).max(46.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 62.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.accent_soft);
    }
    let icon_rect = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 18.0), vec2(26.0, 26.0));
    crate::icons::paint(ui.painter(), icon_rect, icon, t.accent);
    ui.painter().text(egui::pos2(rect.center().x, rect.top() + 44.0), egui::Align2::CENTER_TOP, label, egui::FontId::proportional(11.0), t.text);
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// A small ribbon button: icon and caption on one line.
pub fn small_button(ui: &mut Ui, t: &Tokens, icon: &str, label: &str, tip: &str) -> Response {
    let w = text_w(ui, label) + 30.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 20.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, t.accent_soft);
    }
    if !icon.is_empty() {
        crate::icons::paint(ui.painter(), egui::Rect::from_min_size(rect.min + vec2(3.0, 2.0), vec2(16.0, 16.0)), icon, t.accent);
    }
    ui.painter().text(rect.min + vec2(24.0, 10.0), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(11.5), t.text);
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

pub fn text_w(ui: &Ui, s: &str) -> f32 {
    ui.ctx().fonts_mut(|f| f.layout_no_wrap(s.to_string(), egui::FontId::proportional(11.5), Color32::BLACK).size().x)
}

/// A ribbon group: contents with a caption underneath and a separator on the right.
pub fn group(ui: &mut Ui, t: &Tokens, caption: &str, add: impl FnOnce(&mut Ui)) {
    let top = ui.cursor().min;
    let inner = ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 66.0), egui::Layout::left_to_right(egui::Align::Min), |ui| {
        ui.set_min_height(64.0);
        add(ui);
    });
    let w = inner.response.rect.width().max(text_w(ui, caption) + 8.0);
    ui.painter().text(egui::pos2(top.x + w / 2.0, top.y + 70.0), egui::Align2::CENTER_TOP, caption, egui::FontId::proportional(10.0), t.muted);
    let x = top.x + w + 6.0;
    ui.painter().line_segment([egui::pos2(x, top.y + 4.0), egui::pos2(x, top.y + 80.0)], Stroke::new(1.0, t.border));
    ui.add_space((w - inner.response.rect.width()).max(0.0) + 12.0);
}

/// Stacks small buttons in a column of up to three.
pub fn column(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        add(ui);
    });
}

/// A pane title bar with an optional close button; returns true when closed.
pub fn pane_header(ui: &mut Ui, t: &Tokens, title: &str) -> bool {
    let mut closed = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).strong().size(13.0).color(t.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new("✕").frame(false)).on_hover_text("Close pane").clicked() {
                closed = true;
            }
        });
    });
    ui.add_space(2.0);
    closed
}

/// A chip showing a field in a well.
pub fn chip(ui: &mut Ui, t: &Tokens, text: &str, measure: bool, selected: bool) -> Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 22.0), Sense::click_and_drag());
    let fill = if selected || resp.hovered() { t.accent_soft } else { t.panel_alt };
    ui.painter().rect(rect, CornerRadius::same(4), fill, Stroke::new(1.0, t.border), StrokeKind::Inside);
    let icon = if measure { "∑" } else { "▤" };
    ui.painter().text(rect.left_center() + vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, icon, egui::FontId::proportional(11.0), if measure { t.good } else { t.muted });
    let shown = vizardcraft_core::scene::ellipsize(text, 11.5, w - 50.0);
    ui.painter().text(rect.left_center() + vec2(22.0, 0.0), egui::Align2::LEFT_CENTER, shown, egui::FontId::proportional(11.5), t.text);
    ui.painter().text(rect.right_center() - vec2(8.0, 0.0), egui::Align2::RIGHT_CENTER, "⌄", egui::FontId::proportional(11.0), t.muted);
    resp
}

/// A section header that toggles.
pub fn section(ui: &mut Ui, id: &str, title: &str, default_open: bool, add: impl FnOnce(&mut Ui)) {
    egui::CollapsingHeader::new(egui::RichText::new(title).strong()).id_salt(id).default_open(default_open).show(ui, add);
}

/// A colour button bound to a core colour.
pub fn color_edit(ui: &mut Ui, c: &mut vizardcraft_core::color::Color) -> Response {
    let mut c32 = crate::theme::c32(*c);
    let r = egui::color_picker::color_edit_button_srgba(ui, &mut c32, egui::color_picker::Alpha::OnlyBlend);
    if r.changed() {
        *c = crate::theme::from32(c32);
    }
    r
}
