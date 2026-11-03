//! UI colours and the egui style.

use egui::{Color32, CornerRadius, Stroke, Visuals};

#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub bg: Color32,
    pub panel: Color32,
    pub panel_alt: Color32,
    pub canvas: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub selection: Color32,
    pub danger: Color32,
    pub good: Color32,
    pub ribbon: Color32,
    pub rail: Color32,
    pub rail_text: Color32,
    pub dark: bool,
}

pub const LIGHT: Tokens = Tokens {
    bg: Color32::from_rgb(0xF5, 0xF6, 0xF8),
    panel: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    panel_alt: Color32::from_rgb(0xF7, 0xF8, 0xFA),
    canvas: Color32::from_rgb(0xE9, 0xEC, 0xF1),
    border: Color32::from_rgb(0xDF, 0xE3, 0xE8),
    text: Color32::from_rgb(0x1F, 0x23, 0x2B),
    muted: Color32::from_rgb(0x6B, 0x72, 0x80),
    accent: Color32::from_rgb(0x2B, 0x6C, 0xE0),
    accent_soft: Color32::from_rgb(0xE3, 0xED, 0xFD),
    selection: Color32::from_rgb(0x2B, 0x6C, 0xE0),
    danger: Color32::from_rgb(0xDC, 0x26, 0x26),
    good: Color32::from_rgb(0x16, 0xA3, 0x4A),
    ribbon: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    rail: Color32::from_rgb(0x1B, 0x22, 0x33),
    rail_text: Color32::from_rgb(0xC9, 0xD2, 0xE3),
    dark: false,
};

pub const DARK: Tokens = Tokens {
    bg: Color32::from_rgb(0x15, 0x18, 0x1F),
    panel: Color32::from_rgb(0x1D, 0x21, 0x2A),
    panel_alt: Color32::from_rgb(0x23, 0x28, 0x33),
    canvas: Color32::from_rgb(0x10, 0x12, 0x17),
    border: Color32::from_rgb(0x30, 0x36, 0x44),
    text: Color32::from_rgb(0xE6, 0xE9, 0xEF),
    muted: Color32::from_rgb(0x94, 0x9C, 0xAB),
    accent: Color32::from_rgb(0x5B, 0x9B, 0xFF),
    accent_soft: Color32::from_rgb(0x22, 0x33, 0x55),
    selection: Color32::from_rgb(0x5B, 0x9B, 0xFF),
    danger: Color32::from_rgb(0xF8, 0x71, 0x71),
    good: Color32::from_rgb(0x4A, 0xDE, 0x80),
    ribbon: Color32::from_rgb(0x1D, 0x21, 0x2A),
    rail: Color32::from_rgb(0x0E, 0x11, 0x18),
    rail_text: Color32::from_rgb(0xA9, 0xB4, 0xC8),
    dark: true,
};

pub fn tokens(ctx: &egui::Context) -> Tokens {
    if ctx.global_style().visuals.dark_mode { DARK } else { LIGHT }
}

pub fn apply(ctx: &egui::Context, dark: bool) {
    let t = if dark { DARK } else { LIGHT };
    let mut v = if dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = t.panel;
    v.window_fill = t.panel;
    v.extreme_bg_color = if dark { Color32::from_rgb(0x12, 0x15, 0x1B) } else { Color32::WHITE };
    v.faint_bg_color = t.panel_alt;
    v.selection.bg_fill = t.accent_soft;
    v.selection.stroke = Stroke::new(1.0, t.accent);
    v.hyperlink_color = t.accent;
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    v.window_stroke = Stroke::new(1.0, t.border);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.noninteractive, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(4);
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.inactive.weak_bg_fill = if dark { t.panel_alt } else { Color32::from_rgb(0xF1, 0xF3, 0xF6) };
    v.widgets.inactive.bg_fill = v.widgets.inactive.weak_bg_fill;
    v.widgets.hovered.weak_bg_fill = t.accent_soft;
    v.widgets.hovered.bg_fill = t.accent_soft;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.accent);
    v.widgets.active.weak_bg_fill = t.accent_soft;
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 5.0);
        s.spacing.button_padding = egui::vec2(8.0, 3.0);
        s.spacing.interact_size.y = 22.0;
        s.spacing.menu_margin = egui::Margin::same(6);
        s.interaction.tooltip_delay = 0.35;
    });
}

pub fn c32(c: vizcraft_core::color::Color) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

pub fn from32(c: Color32) -> vizcraft_core::color::Color {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    vizcraft_core::color::Color::rgba(r, g, b, a)
}
