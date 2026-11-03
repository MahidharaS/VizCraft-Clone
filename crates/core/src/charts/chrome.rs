//! The element container: background, border, shadow, title and subtitle.

use crate::color::{Color, Theme};
use crate::doc::*;
use crate::fx::Props;
use crate::scene::{Align, Line, Rect, Scene, Shape, ellipsize};

pub struct Chrome {
    /// Background items (drawn under the content).
    pub scene: Scene,
    /// The content box, relative to the element's top-left.
    pub content: Rect,
}

pub fn radius(el: &Element, props: &Props, theme: &Theme) -> f32 {
    let r = props.f32("border.radius", -1.0);
    if r >= 0.0 {
        r
    } else if el.style.radius.first().copied().unwrap_or(-1.0) >= 0.0 {
        el.style.radius.first().copied().unwrap_or(0.0)
    } else {
        theme.corner_radius
    }
}

/// The resolved title text of a visual: the fx/template title, or a generated one
/// ("Total Sales by Region").
pub fn title_text(el: &Element, props: &Props) -> String {
    let t = props.text("title.text", "");
    if !t.is_empty() {
        return t;
    }
    if let Some(v) = el.visual() {
        let vals: Vec<String> = ["values", "x", "y", "field", "columns"].iter().flat_map(|r| v.well(r)).filter(|f| f.is_value() || v.kind == VisualKind::Slicer || v.kind == VisualKind::Table).map(|f| f.label()).take(3).collect();
        let cats: Vec<String> = ["axis", "legend", "rows", "location", "details"].iter().flat_map(|r| v.well(r)).filter(|f| !f.is_value()).map(|f| f.label()).take(2).collect();
        if v.kind == VisualKind::Slicer || v.kind == VisualKind::Table {
            return if v.kind == VisualKind::Slicer { vals.first().cloned().unwrap_or_default() } else { String::new() };
        }
        if vals.is_empty() {
            return String::new();
        }
        let mut s = vals.join(" and ");
        if !cats.is_empty() {
            s += " by ";
            s += &cats.join(" and ");
        }
        return s;
    }
    String::new()
}

pub fn chrome(el: &Element, props: &Props, theme: &Theme) -> Chrome {
    let mut s = Scene::new(el.w, el.h);
    let is_visual = el.visual().is_some();
    let r = radius(el, props, theme);
    let bg_on = props.bool("background.show", is_visual);
    if is_visual && theme.shadow && props.bool("shadow.show", true) || props.bool("shadow.show", false) {
        let sc = props.color("shadow.color", Color::rgba(0, 0, 0, 28));
        let off = props.f32("shadow.offset", 2.0);
        let blur = props.f32("shadow.blur", 8.0);
        for i in 0..4 {
            let g = blur * (i as f32 + 1.0) / 4.0;
            s.push(Shape::Rect { rect: Rect::new(-g / 2.0, off - g / 2.0 + 1.0, el.w + g, el.h + g), fill: Some(sc.with_alpha((sc.a as f32 / (5.0 + i as f32 * 2.0)) as u8)), stroke: None, radius: r + g });
        }
    }
    if bg_on {
        let mut bg = props.color_opt("background.color").or(el.style.fill.as_ref().and_then(Fill::color)).unwrap_or(theme.visual_background);
        let tr = props.f32("background.transparency", 0.0);
        if tr > 0.0 {
            bg = bg.with_alpha(((100.0 - tr.clamp(0.0, 100.0)) / 100.0 * bg.a as f32) as u8);
        }
        let border = if props.bool("border.show", false) || el.style.stroke.width > 0.0 {
            Some(Line::solid(props.f32("border.width", el.style.stroke.width.max(1.0)), props.color_opt("border.color").unwrap_or(if el.style.stroke.width > 0.0 { el.style.stroke.color } else { theme.visual_border })))
        } else if is_visual {
            Some(Line::solid(1.0, theme.visual_border))
        } else {
            None
        };
        s.push(Shape::Rect { rect: Rect::new(0.0, 0.0, el.w, el.h), fill: Some(bg), stroke: border, radius: r });
    }
    let pad = el.style.padding.max(0.0);
    let mut top = pad;
    if is_visual && props.bool("title.show", true) {
        let title = title_text(el, props);
        if !title.is_empty() {
            let size = props.f32("title.size", theme.title_size);
            let color = props.color_opt("title.color").unwrap_or(theme.foreground);
            if let Some(tb) = props.color_opt("title.background") {
                s.rect(Rect::new(0.0, 0.0, el.w, size * 1.6 + pad), tb);
            }
            let (x, h) = match props.text("title.align", "Left").as_str() {
                "Center" => (el.w / 2.0, Align::Center),
                "Right" => (el.w - pad, Align::End),
                _ => (pad + 2.0, Align::Start),
            };
            s.push(Shape::Text { x, y: top, text: ellipsize(&title, size, el.w - 2.0 * pad - 40.0), size, color, bold: props.bool("title.bold", true), italic: false, h, v: Align::Start, angle: 0.0 });
            top += size * 1.45;
            let sub = props.text("subtitle.text", "");
            if !sub.is_empty() {
                s.push(Shape::Text { x, y: top, text: ellipsize(&sub, size * 0.8, el.w - 2.0 * pad), size: size * 0.8, color: theme.foreground_muted, bold: false, italic: false, h, v: Align::Start, angle: 0.0 });
                top += size * 1.2;
            }
            if props.bool("divider.show", false) {
                s.line((pad, top), (el.w - pad, top), Line::solid(1.0, theme.visual_border));
                top += 4.0;
            }
            top += 4.0;
        }
    }
    Chrome { scene: s, content: Rect::new(pad, top, (el.w - 2.0 * pad).max(0.0), (el.h - top - pad).max(0.0)) }
}
