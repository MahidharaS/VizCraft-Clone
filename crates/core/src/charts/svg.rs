//! Scenes as SVG.

use crate::scene::{Align, Item, Scene, Shape};

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn fill_attr(c: Option<crate::color::Color>) -> String {
    match c {
        Some(c) if c.a > 0 => format!(" fill=\"{}\"", c.css()),
        _ => " fill=\"none\"".into(),
    }
}

fn stroke_attr(l: &Option<crate::scene::Line>) -> String {
    match l {
        Some(l) if l.width > 0.0 => {
            let dash = l.dash.map(|(a, b)| format!(" stroke-dasharray=\"{a},{b}\"")).unwrap_or_default();
            format!(" stroke=\"{}\" stroke-width=\"{}\"{dash} stroke-linejoin=\"round\" stroke-linecap=\"round\"", l.color.css(), l.width)
        }
        _ => String::new(),
    }
}

pub fn arc_path(cx: f32, cy: f32, ri: f32, ro: f32, a0: f32, a1: f32) -> String {
    let pt = |r: f32, a: f32| (cx + r * a.sin(), cy - r * a.cos());
    let large = if a1 - a0 > std::f32::consts::PI { 1 } else { 0 };
    let (x0, y0) = pt(ro, a0);
    let (x1, y1) = pt(ro, a1);
    if (a1 - a0) >= std::f32::consts::TAU - 1e-4 {
        // Full ring: two half arcs.
        let (xm, ym) = pt(ro, a0 + std::f32::consts::PI);
        let mut d = format!("M{x0:.2},{y0:.2} A{ro},{ro} 0 1 1 {xm:.2},{ym:.2} A{ro},{ro} 0 1 1 {x0:.2},{y0:.2} Z");
        if ri > 0.0 {
            let (a, b) = pt(ri, a0);
            let (c, e) = pt(ri, a0 + std::f32::consts::PI);
            d += &format!(" M{a:.2},{b:.2} A{ri},{ri} 0 1 0 {c:.2},{e:.2} A{ri},{ri} 0 1 0 {a:.2},{b:.2} Z");
        }
        return d;
    }
    if ri <= 0.0 {
        return format!("M{cx:.2},{cy:.2} L{x0:.2},{y0:.2} A{ro},{ro} 0 {large} 1 {x1:.2},{y1:.2} Z");
    }
    let (x2, y2) = pt(ri, a1);
    let (x3, y3) = pt(ri, a0);
    format!("M{x0:.2},{y0:.2} A{ro},{ro} 0 {large} 1 {x1:.2},{y1:.2} L{x2:.2},{y2:.2} A{ri},{ri} 0 {large} 0 {x3:.2},{y3:.2} Z")
}

fn item_svg(it: &Item) -> String {
    let mut attrs = String::new();
    if let Some(h) = &it.hit {
        let ident: Vec<serde_json::Value> = h.point.iter().map(|(f, v)| serde_json::json!([f, v])).collect();
        attrs += &format!(" class=\"vc-hit\" data-hit=\"{}\"", esc(&serde_json::to_string(&ident).unwrap_or_default()));
        if !h.tooltip.is_empty() {
            let tip: Vec<String> = h.tooltip.iter().map(|(k, v)| format!("{k}: {v}")).collect();
            attrs += &format!(" data-tip=\"{}\"", esc(&tip.join("\n")));
        }
    }
    if it.dimmed {
        attrs += " opacity=\"0.35\"";
    }
    match &it.shape {
        Shape::Rect { rect, fill, stroke, radius } => format!(
            "<rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" rx=\"{radius}\"{}{}{attrs}/>",
            rect.x,
            rect.y,
            rect.w.max(0.0),
            rect.h.max(0.0),
            fill_attr(*fill),
            stroke_attr(stroke)
        ),
        Shape::Path { points, closed, fill, stroke } => {
            let d: Vec<String> = points.iter().enumerate().map(|(i, (x, y))| format!("{}{x:.2},{y:.2}", if i == 0 { "M" } else { "L" })).collect();
            format!("<path d=\"{}{}\"{}{}{attrs}/>", d.join(" "), if *closed { " Z" } else { "" }, if *closed { fill_attr(*fill) } else { " fill=\"none\"".into() }, stroke_attr(stroke))
        }
        Shape::Circle { cx, cy, r, fill, stroke } => format!("<circle cx=\"{cx:.2}\" cy=\"{cy:.2}\" r=\"{r:.2}\"{}{}{attrs}/>", fill_attr(*fill), stroke_attr(stroke)),
        Shape::Arc { cx, cy, r_inner, r_outer, start, end, fill, stroke } => {
            format!("<path d=\"{}\"{}{}{attrs}/>", arc_path(*cx, *cy, *r_inner, *r_outer, *start, *end), fill_attr(Some(*fill)), stroke_attr(stroke))
        }
        Shape::Text { x, y, text, size, color, bold, italic, h, v, angle } => {
            let anchor = match h {
                Align::Start => "start",
                Align::Center => "middle",
                Align::End => "end",
            };
            let baseline = match v {
                Align::Start => "hanging",
                Align::Center => "central",
                Align::End => "alphabetic",
            };
            let rot = if *angle != 0.0 { format!(" transform=\"rotate({angle} {x:.2} {y:.2})\"") } else { String::new() };
            format!(
                "<text x=\"{x:.2}\" y=\"{y:.2}\" font-size=\"{size}\" fill=\"{}\" text-anchor=\"{anchor}\" dominant-baseline=\"{baseline}\"{}{}{rot}{attrs}>{}</text>",
                color.css(),
                if *bold { " font-weight=\"600\"" } else { "" },
                if *italic { " font-style=\"italic\"" } else { "" },
                esc(text)
            )
        }
        Shape::Image { rect, png_base64 } => format!(
            "<image x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" href=\"data:image/png;base64,{png_base64}\" preserveAspectRatio=\"xMidYMid meet\"{attrs}/>",
            rect.x, rect.y, rect.w, rect.h
        ),
    }
}

pub fn scene_svg(s: &Scene) -> String {
    let mut out = String::from("<g>");
    for it in &s.items {
        out += &item_svg(it);
    }
    if let Some(m) = &s.message {
        out += &format!("<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"12\" fill=\"#888\" text-anchor=\"middle\" dominant-baseline=\"central\">{}</text>", s.width / 2.0, s.height / 2.0, esc(m));
    }
    out += "</g>";
    out
}

/// A standalone SVG document of a scene.
pub fn document(s: &Scene) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\" font-family=\"Inter, Segoe UI, Helvetica, Arial, sans-serif\">{}</svg>",
        scene_svg(s),
        w = s.width,
        h = s.height
    )
}
