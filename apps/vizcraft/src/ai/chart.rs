//! Draws a visual spec + its data with the core chart renderers (used by the Formulator's
//! thread cards and main chart).

use egui::{Rect, Ui};
use vizcraft_core::charts::{self, RenderOpts};
use vizcraft_core::color::Theme;
use vizcraft_core::doc::VisualSpec;
use vizcraft_core::fx::Props;
use vizcraft_core::scene::Scene;
use vizcraft_core::vquery::VisualData;

use crate::paint::{Xf, paint_scene};
use crate::theme::c32;

pub fn scene(theme: &Theme, spec: &VisualSpec, data: &VisualData, w: f32, h: f32, compact: bool) -> Scene {
    let mut props = Props::default();
    if compact {
        for k in ["legend.show", "title.show"] {
            props.map.insert(k.into(), vizcraft_core::doc::Prop::Bool(false));
        }
    }
    let o = RenderOpts { theme, props: &props, width: w, height: h, selected: &[], analytics: &[], script_image: None, search: "" };
    charts::render(spec, data, &o)
}

/// Paints a chart card into `rect` (background, border, content).
pub fn paint(ui: &Ui, rect: Rect, theme: &Theme, spec: &VisualSpec, data: &VisualData, compact: bool) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, c32(theme.visual_background));
    let pad = if compact { 4.0 } else { 12.0 };
    let inner = rect.shrink(pad);
    if inner.width() < 20.0 || inner.height() < 20.0 {
        return;
    }
    let s = scene(theme, spec, data, inner.width(), inner.height(), compact);
    paint_scene(&painter, &s, Xf { origin: inner.min, scale: 1.0 });
}
