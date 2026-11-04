//! Rendering report elements — visuals, shapes, text boxes, images and buttons — the same way
//! everywhere: as a [`Scene`] (for renderers that paint scenes, like the desktop) and as SVG
//! (the web viewer, thumbnails, `vizardcraft-server render`, SVG export).
//!
//! [`render_element`] composes the element container ([`chrome`]) with its content at the right
//! offset. [`element_svg`] does the same as a standalone `<svg>` and adds what scenes can't
//! express: gradient fills, drop shadows, clipping, hover fills, rich text with mixed runs.

use std::fmt::Write as _;

use vizardcraft_core::analytics;
use vizardcraft_core::build::Built;
use vizardcraft_core::charts::{self, RenderOpts, chrome};
use vizardcraft_core::color::{Color, Theme};
use vizardcraft_core::doc::*;
use vizardcraft_core::fx::{self, FxEnv, Props};
use vizardcraft_core::scalar::Scalar;
use vizardcraft_core::scene::{Align, Line, Rect, Scene, Shape, text_width};
use vizardcraft_core::state::{self, Runtime};
use vizardcraft_core::vquery;

/// The font stack published reports use.
pub const FONT_STACK: &str = "Inter, 'Segoe UI', system-ui, -apple-system, Helvetica, Arial, sans-serif";

// ---------------------------------------------------------------------------------------------
// Visuals
// ---------------------------------------------------------------------------------------------

/// A rendered visual: the container, the content box and the content (in content-box coordinates).
pub struct VisualRender {
    pub chrome: Scene,
    pub content_box: Rect,
    pub content: Scene,
    /// What filters the visual (human readable).
    pub filters: Vec<String>,
    /// The visual's axis has more than one level (drill up/down applies).
    pub drillable: bool,
}

/// Renders a visual element in its filter context.
pub fn render_visual(p: &Project, built: &Built, rt: &Runtime, page: &Page, el: &Element) -> Option<VisualRender> {
    let v = el.visual()?;
    let cx = state::context_for(p, built, rt, page, el);
    let env = FxEnv::new(p, built, &cx, &page.name);
    let mut props = fx::resolve_props(&env, el);
    let data = vquery::run(p, built, v, &cx);
    fx::resolve_point_colors(&env, el, &data, &mut props);
    fx::resolve_cell_colors(&env, el, &data, &mut props);
    let theme = &p.report.theme;
    let ch = chrome::chrome(el, &props, theme);
    let computed = analytics::compute(&v.analytics, &data);
    let selected: Vec<Vec<(FieldRef, Scalar)>> = rt.selection.as_ref().filter(|s| s.source == el.id).map(|s| s.points.clone()).unwrap_or_default();
    let o = RenderOpts { theme, props: &props, width: ch.content.w, height: ch.content.h, selected: &selected, analytics: &computed, script_image: None, search: "" };
    let content = charts::render(v, &data, &o);
    Some(VisualRender { chrome: ch.scene, content_box: ch.content, content, filters: cx.description.clone(), drillable: data.drill_levels > 1 })
}

/// Moves every item (and action region) of a scene by `(dx, dy)`.
pub fn offset_scene(s: &mut Scene, dx: f32, dy: f32) {
    for it in &mut s.items {
        offset_shape(&mut it.shape, dx, dy);
    }
    for (r, _) in &mut s.actions {
        r.x += dx;
        r.y += dy;
    }
}

pub fn offset_shape(s: &mut Shape, dx: f32, dy: f32) {
    match s {
        Shape::Rect { rect, .. } | Shape::Image { rect, .. } => {
            rect.x += dx;
            rect.y += dy;
        }
        Shape::Path { points, .. } => {
            for p in points {
                p.0 += dx;
                p.1 += dy;
            }
        }
        Shape::Circle { cx, cy, .. } | Shape::Arc { cx, cy, .. } => {
            *cx += dx;
            *cy += dy;
        }
        Shape::Text { x, y, .. } => {
            *x += dx;
            *y += dy;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The one entry point: an element as a scene
// ---------------------------------------------------------------------------------------------

/// Renders any element as a scene in element coordinates (`0..el.w × 0..el.h`): the container
/// (background, border, shadow, title) plus the content. Gradients are approximated by their
/// first colour; [`element_svg`] renders them exactly.
pub fn render_element(p: &Project, built: &Built, rt: &Runtime, page: &Page, el: &Element) -> Scene {
    let theme = &p.report.theme;
    if let Some(vr) = render_visual(p, built, rt, page, el) {
        let mut s = vr.chrome;
        let mut c = vr.content;
        offset_scene(&mut c, vr.content_box.x, vr.content_box.y);
        s.items.extend(c.items);
        s.actions.extend(c.actions);
        if s.message.is_none() {
            s.message = c.message;
        }
        return s;
    }
    let cx = state::context_for(p, built, rt, page, el);
    let env = FxEnv::new(p, built, &cx, &page.name);
    let props = object_props(&env, el);
    let ch = chrome::chrome(el, &props, theme);
    let mut s = ch.scene;
    match &el.kind {
        ElementKind::Shape(sp) => {
            let st = shape_style(sp, &props, el);
            let fill = st.fill.color();
            let stroke = if st.stroke_w > 0.0 { Some(Line::from_style(st.stroke_w, st.stroke_color, &sp.stroke.dash)) } else { None };
            match outline(sp, el.w, el.h, st.radius) {
                Outline::Rect(r) => s.push(Shape::Rect { rect: Rect::new(0.0, 0.0, el.w, el.h), fill, stroke, radius: r }),
                Outline::Ellipse => {
                    let pts = ellipse_points(el.w, el.h, 72);
                    s.push(Shape::Path { points: pts, closed: true, fill, stroke });
                }
                Outline::Line(a, b) => s.line(a, b, Line::from_style(st.stroke_w.max(2.0), if st.stroke_w > 0.0 { st.stroke_color } else { fill.unwrap_or(theme.foreground) }, &sp.stroke.dash)),
                Outline::Poly(pts, closed) => s.push(Shape::Path { points: pts, closed, fill: if closed { fill } else { None }, stroke: stroke.or(if closed { None } else { Some(Line::solid(2.0, fill.unwrap_or(theme.foreground))) }) }),
            }
            let text = shape_text(sp, &props);
            if !text.is_empty() {
                let lines = layout_rich(&env, &text, theme, el.w - 16.0, el.h - 8.0, default_text_color(&st.fill, theme));
                scene_text(&mut s, &lines, &text, Rect::new(8.0, 4.0, el.w - 16.0, el.h - 8.0));
            }
        }
        ElementKind::Text(t) => {
            let rich = text_rich(t, &props);
            let pad = el.style.padding.max(0.0).min(el.w / 4.0);
            let bx = Rect::new(pad, pad, (el.w - 2.0 * pad).max(1.0), (el.h - 2.0 * pad).max(1.0));
            let lines = layout_rich(&env, &rich, theme, bx.w, bx.h, theme.foreground);
            scene_text(&mut s, &lines, &rich, bx);
        }
        ElementKind::Image(img) => {
            if let Some((mime, b64)) = image_data(p, &img_source(img, &props))
                && mime == "image/png"
            {
                s.push(Shape::Image { rect: Rect::new(0.0, 0.0, el.w, el.h), png_base64: b64 });
            } else {
                s.push(Shape::Rect { rect: Rect::new(0.0, 0.0, el.w, el.h), fill: Some(Color::rgb(0xEE, 0xF0, 0xF3)), stroke: None, radius: 4.0 });
            }
        }
        ElementKind::Button(b) => {
            let bs = button_style(b, &props, el);
            s.push(Shape::Rect { rect: Rect::new(0.0, 0.0, el.w, el.h), fill: bs.fill.color(), stroke: if el.style.stroke.width > 0.0 { Some(Line::solid(el.style.stroke.width, el.style.stroke.color)) } else { None }, radius: bs.radius });
            let lines = layout_rich(&env, &bs.text, theme, el.w - 12.0, el.h - 4.0, bs.text_color);
            scene_text(&mut s, &lines, &bs.text, Rect::new(6.0, 2.0, el.w - 12.0, el.h - 4.0));
        }
        ElementKind::Group | ElementKind::Visual(_) => {}
    }
    s
}

// ---------------------------------------------------------------------------------------------
// Element properties for non-visual elements
// ---------------------------------------------------------------------------------------------

/// Resolved props of a shape / text / image / button. The schema turns the container background
/// on for every kind; objects only get one when the user asked for it (a fill in the box style or
/// an explicit property), so text over a banner stays transparent.
fn object_props(env: &FxEnv, el: &Element) -> Props {
    let mut props = fx::resolve_props(env, el);
    let explicit = |k: &str| el.props.contains_key(k);
    if !explicit("background.show") {
        props.map.insert("background.show".into(), Prop::Bool(el.style.fill.as_ref().is_some_and(|f| !matches!(f, Fill::None))));
    }
    if !explicit("shadow.show") {
        props.map.insert("shadow.show".into(), Prop::Bool(el.style.shadow.enabled));
    }
    props
}

struct ShapeStyle {
    fill: Fill,
    stroke_w: f32,
    stroke_color: Color,
    radius: f32,
}

fn shape_style(sp: &ShapeSpec, props: &Props, el: &Element) -> ShapeStyle {
    let mut fill = match props.color_opt("shape.fill") {
        Some(c) => Fill::Solid(c),
        None => sp.fill.clone(),
    };
    let tr = props.f32("shape.fillTransparency", 0.0).clamp(0.0, 100.0);
    if tr > 0.0 {
        let a = |c: Color| c.with_alpha(((100.0 - tr) / 100.0 * c.a as f32) as u8);
        fill = match fill {
            Fill::Solid(c) => Fill::Solid(a(c)),
            Fill::Linear { angle, stops } => Fill::Linear { angle, stops: stops.into_iter().map(|(t, c)| (t, a(c))).collect() },
            Fill::Radial { stops } => Fill::Radial { stops: stops.into_iter().map(|(t, c)| (t, a(c))).collect() },
            Fill::None => Fill::None,
        };
    }
    let sw = props.f32("shape.strokeWidth", -1.0);
    let stroke_w = if sw >= 0.0 { sw } else { sp.stroke.width };
    let stroke_color = props.color_opt("shape.stroke").unwrap_or(sp.stroke.color);
    let r = props.f32("shape.radius", -1.0);
    let radius = if r >= 0.0 {
        r
    } else if el.style.radius.first().copied().unwrap_or(-1.0) >= 0.0 {
        el.style.radius.first().copied().unwrap_or(0.0)
    } else if sp.shape == ShapeKind::RoundedRectangle {
        (el.w.min(el.h) * 0.18).min(16.0)
    } else {
        0.0
    };
    ShapeStyle { fill, stroke_w, stroke_color, radius }
}

fn shape_text(sp: &ShapeSpec, props: &Props) -> RichText {
    let t = props.text("shape.text", "");
    if !t.is_empty() {
        let mut r = RichText::plain(&t);
        if let Some(run) = r.runs.first_mut() {
            run.size = props.f32("shape.textSize", 14.0);
            run.color = props.color_opt("shape.textColor").map(Prop::Color);
        }
        r.align = props.text("shape.textAlign", "Center").to_lowercase();
        r.valign = "middle".into();
        return r;
    }
    let mut r = sp.text.clone();
    if r.align.is_empty() {
        r.align = "center".into();
    }
    if r.valign.is_empty() {
        r.valign = "middle".into();
    }
    r
}

fn text_rich(t: &TextSpec, props: &Props) -> RichText {
    let o = props.text("text.override", "");
    let mut r = if o.is_empty() { t.text.clone() } else { RichText { runs: vec![TextRun { text: o, ..t.text.runs.first().cloned().unwrap_or_default() }], ..t.text.clone() } };
    if r.valign.is_empty() {
        r.valign = props.text("text.valign", "Top").to_lowercase();
    }
    r
}

struct ButtonStyle {
    fill: Fill,
    hover: Fill,
    radius: f32,
    text: RichText,
    text_color: Color,
    icon: String,
}

fn button_style(b: &ButtonSpec, props: &Props, el: &Element) -> ButtonStyle {
    let fill = props.color_opt("button.fill").map(Fill::Solid).unwrap_or_else(|| b.fill.clone());
    let hover = props.color_opt("button.hoverFill").map(Fill::Solid).unwrap_or_else(|| b.hover_fill.clone());
    let radius = if el.style.radius.first().copied().unwrap_or(-1.0) >= 0.0 { el.style.radius.first().copied().unwrap_or(6.0) } else { props.f32("button.radius", 6.0) };
    let fallback = fill.color().map(|c| if c.a < 60 { Color::rgb(0x1F, 0x23, 0x2B) } else { c.contrast_text() }).unwrap_or(Color::rgb(0x1F, 0x23, 0x2B));
    let text_color = props.color_opt("button.textColor").unwrap_or(fallback);
    let over = props.text("button.text", "");
    let mut text = if over.is_empty() { b.text.clone() } else { RichText::plain(&over) };
    if !over.is_empty()
        && let Some(r) = text.runs.first_mut()
    {
        r.size = props.f32("button.textSize", 12.0);
    }
    text.align = if text.align.is_empty() { "center".into() } else { text.align.clone() };
    text.valign = "middle".into();
    let icon = match props.text("button.icon", "None").as_str() {
        "None" => b.icon.clone(),
        other => other.to_string(),
    };
    ButtonStyle { fill, hover, radius, text, text_color, icon }
}

fn img_source(img: &ImageSpec, props: &Props) -> String {
    let u = props.text("image.url", "");
    if u.is_empty() { img.source.clone() } else { u }
}

/// The MIME type and base64 data of an embedded image (`report.images`), sniffed from the data.
pub fn image_data(p: &Project, source: &str) -> Option<(&'static str, String)> {
    let b64 = p.report.images.get(source)?;
    let b64 = b64.split_once("base64,").map(|(_, d)| d).unwrap_or(b64).trim().to_string();
    let mime = if b64.starts_with("iVBOR") {
        "image/png"
    } else if b64.starts_with("/9j/") {
        "image/jpeg"
    } else if b64.starts_with("R0lG") {
        "image/gif"
    } else if b64.starts_with("PHN2") || b64.starts_with("PD94") {
        "image/svg+xml"
    } else if b64.starts_with("UklG") {
        "image/webp"
    } else {
        "image/png"
    };
    Some((mime, b64))
}

fn default_text_color(fill: &Fill, theme: &Theme) -> Color {
    match fill.color() {
        Some(c) if c.a > 120 => c.contrast_text(),
        _ => theme.foreground,
    }
}

// ---------------------------------------------------------------------------------------------
// Shape outlines
// ---------------------------------------------------------------------------------------------

enum Outline {
    Rect(f32),
    Ellipse,
    Line((f32, f32), (f32, f32)),
    Poly(Vec<(f32, f32)>, bool),
}

fn regular(n: usize, w: f32, h: f32, rot: f32) -> Vec<(f32, f32)> {
    (0..n)
        .map(|i| {
            let a = rot + i as f32 * std::f32::consts::TAU / n as f32;
            (w / 2.0 + w / 2.0 * a.sin(), h / 2.0 - h / 2.0 * a.cos())
        })
        .collect()
}

fn ellipse_points(w: f32, h: f32, n: usize) -> Vec<(f32, f32)> {
    regular(n, w, h, 0.0)
}

fn outline(sp: &ShapeSpec, w: f32, h: f32, radius: f32) -> Outline {
    let p = |fx: f32, fy: f32| (fx * w, fy * h);
    match sp.shape {
        ShapeKind::Rectangle | ShapeKind::RoundedRectangle | ShapeKind::Frame => Outline::Rect(radius),
        ShapeKind::Ellipse => Outline::Ellipse,
        ShapeKind::Line => Outline::Line((0.0, h / 2.0), (w, h / 2.0)),
        ShapeKind::Arrow => Outline::Poly(vec![p(0.0, 0.32), p(0.66, 0.32), p(0.66, 0.0), p(1.0, 0.5), p(0.66, 1.0), p(0.66, 0.68), p(0.0, 0.68)], true),
        ShapeKind::Triangle => Outline::Poly(vec![p(0.5, 0.0), p(1.0, 1.0), p(0.0, 1.0)], true),
        ShapeKind::Diamond => Outline::Poly(vec![p(0.5, 0.0), p(1.0, 0.5), p(0.5, 1.0), p(0.0, 0.5)], true),
        ShapeKind::Pentagon => Outline::Poly(regular(5, w, h, 0.0), true),
        ShapeKind::Hexagon => Outline::Poly(regular(6, w, h, std::f32::consts::FRAC_PI_6), true),
        ShapeKind::Star => {
            let mut pts = Vec::new();
            for i in 0..10 {
                let a = i as f32 * std::f32::consts::PI / 5.0;
                let r = if i % 2 == 0 { 1.0 } else { 0.42 };
                pts.push((w / 2.0 + w / 2.0 * r * a.sin(), h / 2.0 - h / 2.0 * r * a.cos()));
            }
            Outline::Poly(pts, true)
        }
        ShapeKind::Chevron => Outline::Poly(vec![p(0.0, 0.0), p(0.72, 0.0), p(1.0, 0.5), p(0.72, 1.0), p(0.0, 1.0), p(0.28, 0.5)], true),
        ShapeKind::Speech => Outline::Poly(vec![p(0.0, 0.0), p(1.0, 0.0), p(1.0, 0.78), p(0.36, 0.78), p(0.16, 1.0), p(0.2, 0.78), p(0.0, 0.78)], true),
        ShapeKind::Heart => {
            let mut pts = Vec::new();
            for i in 0..96 {
                let t = i as f32 * std::f32::consts::TAU / 96.0;
                let x = 16.0 * t.sin().powi(3);
                let y = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
                pts.push(((x + 17.0) / 34.0 * w, (13.5 - y) / 30.0 * h));
            }
            Outline::Poly(pts, true)
        }
        ShapeKind::Plus => {
            let (a, b) = (0.33, 0.67);
            Outline::Poly(vec![p(a, 0.0), p(b, 0.0), p(b, a), p(1.0, a), p(1.0, b), p(b, b), p(b, 1.0), p(a, 1.0), p(a, b), p(0.0, b), p(0.0, a), p(a, a)], true)
        }
        ShapeKind::Path => Outline::Poly(sp.points.iter().map(|(x, y)| (x * w, y * h)).collect(), sp.closed),
    }
}

// ---------------------------------------------------------------------------------------------
// Rich text layout
// ---------------------------------------------------------------------------------------------

/// A run piece on one line.
#[derive(Clone, Debug)]
pub struct Seg {
    pub text: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub color: Color,
    pub link: Option<String>,
    pub font: String,
}

#[derive(Clone, Debug, Default)]
pub struct TextLine {
    pub segs: Vec<Seg>,
    pub width: f32,
    pub size: f32,
}

fn seg_width(text: &str, size: f32, bold: bool) -> f32 {
    text_width(text, size) * if bold { 1.06 } else { 1.0 }
}

/// Expands templates and fx colours, then wraps the runs into lines that fit `width`; when they
/// don't fit `height`, the text shrinks (down to 55 %).
pub fn layout_rich(env: &FxEnv, rich: &RichText, theme: &Theme, width: f32, height: f32, default_color: Color) -> Vec<TextLine> {
    // Expand runs once.
    let runs: Vec<Seg> = rich
        .runs
        .iter()
        .map(|r| Seg {
            text: env.template(&r.text),
            size: if r.size > 0.0 { r.size } else { 14.0 },
            bold: r.bold,
            italic: r.italic,
            underline: r.underline,
            color: r.color.as_ref().and_then(|c| env.resolve(c).as_color()).unwrap_or(default_color),
            link: r.link.clone(),
            font: r.font.clone(),
        })
        .collect();
    let lh = if rich.line_height > 0.0 { rich.line_height } else { 1.25 };
    let mut scale = 1.0f32;
    loop {
        let lines = wrap(&runs, width.max(10.0), scale);
        let total: f32 = lines.iter().map(|l| l.size * lh).sum();
        if total <= height + 0.5 || scale <= 0.56 {
            let _ = theme;
            return lines;
        }
        scale *= 0.92;
    }
}

fn wrap(runs: &[Seg], width: f32, scale: f32) -> Vec<TextLine> {
    let mut lines: Vec<TextLine> = vec![TextLine::default()];
    for r in runs {
        let size = r.size * scale;
        for (pi, para) in r.text.split('\n').enumerate() {
            if pi > 0 {
                lines.push(TextLine::default());
            }
            // Words keep their trailing space.
            let mut words: Vec<String> = Vec::new();
            let mut cur = String::new();
            for ch in para.chars() {
                cur.push(ch);
                if ch == ' ' {
                    words.push(std::mem::take(&mut cur));
                }
            }
            if !cur.is_empty() {
                words.push(cur);
            }
            for w in words {
                let ww = seg_width(&w, size, r.bold);
                let Some(line) = lines.last_mut() else { continue };
                if line.width + seg_width(w.trim_end(), size, r.bold) > width && line.width > 0.0 {
                    // Trim the trailing space of the line being closed.
                    if let Some(last) = line.segs.last_mut() {
                        let t = last.text.trim_end().to_string();
                        line.width -= seg_width(&last.text, last.size, last.bold) - seg_width(&t, last.size, last.bold);
                        last.text = t;
                    }
                    lines.push(TextLine::default());
                }
                let Some(line) = lines.last_mut() else { continue };
                line.size = line.size.max(size);
                line.width += ww;
                match line.segs.last_mut() {
                    Some(last) if last.size == size && last.bold == r.bold && last.italic == r.italic && last.color == r.color && last.link == r.link && last.underline == r.underline => last.text.push_str(&w),
                    _ => line.segs.push(Seg { text: w, size, ..r.clone() }),
                }
            }
        }
    }
    for l in &mut lines {
        if l.size == 0.0 {
            l.size = runs.first().map(|r| r.size * scale).unwrap_or(14.0);
        }
    }
    lines
}

fn line_metrics(lines: &[TextLine], rich: &RichText, bx: Rect) -> (Vec<f32>, f32) {
    let lh = if rich.line_height > 0.0 { rich.line_height } else { 1.25 };
    let total: f32 = lines.iter().map(|l| l.size * lh).sum();
    let mut y = match rich.valign.as_str() {
        "middle" | "center" => bx.y + (bx.h - total) / 2.0,
        "bottom" => bx.y + bx.h - total,
        _ => bx.y,
    };
    let mut baselines = Vec::new();
    for l in lines {
        let h = l.size * lh;
        baselines.push(y + (h + l.size * 0.72) / 2.0);
        y += h;
    }
    (baselines, lh)
}

fn scene_text(s: &mut Scene, lines: &[TextLine], rich: &RichText, bx: Rect) {
    let (baselines, _) = line_metrics(lines, rich, bx);
    for (l, base) in lines.iter().zip(baselines) {
        let mut x = match rich.align.as_str() {
            "center" => bx.x + (bx.w - l.width) / 2.0,
            "right" => bx.x + bx.w - l.width,
            _ => bx.x,
        };
        for sg in &l.segs {
            s.push(Shape::Text { x, y: base, text: sg.text.clone(), size: sg.size, color: sg.color, bold: sg.bold, italic: sg.italic, h: Align::Start, v: Align::End, angle: 0.0 });
            x += seg_width(&sg.text, sg.size, sg.bold);
        }
    }
}

fn svg_text(out: &mut String, lines: &[TextLine], rich: &RichText, bx: Rect) {
    let (baselines, _) = line_metrics(lines, rich, bx);
    let (x, anchor) = match rich.align.as_str() {
        "center" => (bx.x + bx.w / 2.0, "middle"),
        "right" => (bx.x + bx.w, "end"),
        _ => (bx.x, "start"),
    };
    for (l, base) in lines.iter().zip(baselines) {
        if l.segs.iter().all(|s| s.text.trim().is_empty()) {
            continue;
        }
        let _ = write!(out, "<text x=\"{x:.1}\" y=\"{base:.1}\" text-anchor=\"{anchor}\" xml:space=\"preserve\" style=\"white-space:pre\">");
        for sg in &l.segs {
            let mut style = format!("font-size:{:.1}px;fill:{}", sg.size, sg.color.css());
            if sg.bold {
                style += ";font-weight:600";
            }
            if sg.italic {
                style += ";font-style:italic";
            }
            if sg.underline || sg.link.is_some() {
                style += ";text-decoration:underline";
            }
            if !sg.font.is_empty() && sg.font != "Inter" {
                let _ = write!(style, ";font-family:'{}', {FONT_STACK}", esc(&sg.font));
            }
            let span = format!("<tspan style=\"{style}\">{}</tspan>", esc(&sg.text));
            match &sg.link {
                Some(url) if safe_url(url) => {
                    let _ = write!(out, "<a href=\"{}\" target=\"_blank\" rel=\"noopener\">{span}</a>", esc(url));
                }
                _ => out.push_str(&span),
            }
        }
        out.push_str("</text>");
    }
}

/// Only http(s), mailto and in-page links are written into published pages.
pub fn safe_url(u: &str) -> bool {
    let l = u.trim().to_ascii_lowercase();
    l.starts_with("https://") || l.starts_with("http://") || l.starts_with("mailto:") || l.starts_with('#')
}

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

// ---------------------------------------------------------------------------------------------
// SVG
// ---------------------------------------------------------------------------------------------

/// An SVG paint for a fill; gradients are added to `defs` with id `id`.
pub fn paint(fill: &Fill, id: &str, defs: &mut String) -> String {
    match fill {
        Fill::None => "none".into(),
        Fill::Solid(c) => c.css(),
        Fill::Linear { angle, stops } => {
            let _ = write!(defs, "<linearGradient id=\"{id}\" gradientTransform=\"rotate({:.1} 0.5 0.5)\">", angle - 90.0);
            for (t, c) in stops {
                let _ = write!(defs, "<stop offset=\"{:.3}\" stop-color=\"{}\"/>", t.clamp(0.0, 1.0), c.css());
            }
            defs.push_str("</linearGradient>");
            format!("url(#{id})")
        }
        Fill::Radial { stops } => {
            let _ = write!(defs, "<radialGradient id=\"{id}\">");
            for (t, c) in stops {
                let _ = write!(defs, "<stop offset=\"{:.3}\" stop-color=\"{}\"/>", t.clamp(0.0, 1.0), c.css());
            }
            defs.push_str("</radialGradient>");
            format!("url(#{id})")
        }
    }
}

/// CSS for a fill (page backgrounds).
pub fn fill_css(fill: &Fill) -> String {
    match fill {
        Fill::None => "transparent".into(),
        Fill::Solid(c) => c.css(),
        Fill::Linear { angle, stops } => {
            let st: Vec<String> = stops.iter().map(|(t, c)| format!("{} {:.1}%", c.css(), t * 100.0)).collect();
            format!("linear-gradient({angle}deg, {})", st.join(", "))
        }
        Fill::Radial { stops } => {
            let st: Vec<String> = stops.iter().map(|(t, c)| format!("{} {:.1}%", c.css(), t * 100.0)).collect();
            format!("radial-gradient(circle, {})", st.join(", "))
        }
    }
}

fn dash_attr(dash: &str, w: f32) -> String {
    match dash {
        "dash" | "dashed" => format!(" stroke-dasharray=\"{:.1},{:.1}\"", w * 4.0, w * 2.5),
        "dot" | "dotted" => format!(" stroke-dasharray=\"{:.1},{:.1}\" stroke-linecap=\"round\"", w * 0.1, w * 2.0),
        _ => String::new(),
    }
}

fn points_attr(pts: &[(f32, f32)]) -> String {
    pts.iter().map(|(x, y)| format!("{x:.2},{y:.2}")).collect::<Vec<_>>().join(" ")
}

fn action_overlays(out: &mut String, s: &Scene) {
    for (r, a) in &s.actions {
        let _ = write!(out, "<rect class=\"vc-act\" x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" fill=\"transparent\" data-action=\"{}\"/>", r.x, r.y, r.w.max(0.0), r.h.max(0.0), esc(a));
    }
}

/// What the viewer needs to know about one rendered element.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ElementOut {
    pub id: u64,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub rot: f32,
    /// "visual", "shape", "text", "image", "button".
    pub kind: String,
    /// The visual kind (serde name) for visuals.
    pub visual: String,
    pub title: String,
    /// A standalone `<svg>` of the element.
    pub svg: String,
    /// The element's click action (buttons, images): an [`Action`] as JSON.
    pub action: Option<serde_json::Value>,
    pub drillable: bool,
    pub drill_mode: bool,
    /// Shows a header with focus mode / drill icons on hover.
    pub header: bool,
}

/// Renders an element as a standalone `<svg>` (`w × h` in its own coordinates), plus the facts
/// the viewer needs. `uid` makes gradient/filter ids unique in the page.
pub fn element_svg(p: &Project, built: &Built, rt: &Runtime, page: &Page, el: &Element, uid: &str) -> ElementOut {
    let theme = &p.report.theme;
    let (w, h) = (el.w.max(1.0), el.h.max(1.0));
    let mut defs = String::new();
    let mut body = String::new();
    let mut out = ElementOut { id: el.id, x: el.x, y: el.y, w: el.w, h: el.h, rot: el.rotation, ..ElementOut::default() };
    if let Some(vr) = render_visual(p, built, rt, page, el) {
        let v = el.visual();
        out.kind = "visual".into();
        out.visual = v.and_then(|v| serde_json::to_value(v.kind).ok()).and_then(|j| j.as_str().map(String::from)).unwrap_or_default();
        out.title = el.name.clone();
        out.drillable = vr.drillable;
        out.drill_mode = v.is_some_and(|v| v.drill.drill_mode);
        out.header = v.is_some_and(|v| v.kind != VisualKind::Slicer);
        body += &vr.chrome.to_svg();
        let cb = vr.content_box;
        let _ = write!(body, "<svg x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" overflow=\"hidden\">", cb.x, cb.y, cb.w.max(0.0), cb.h.max(0.0));
        body += &vr.content.to_svg();
        action_overlays(&mut body, &vr.content);
        body += "</svg>";
    } else {
        let cx = state::context_for(p, built, rt, page, el);
        let env = FxEnv::new(p, built, &cx, &page.name);
        let mut props = object_props(&env, el);
        if el.style.shadow.enabled {
            // An exact SVG drop shadow instead of the scene's stacked rectangles.
            props.map.insert("shadow.show".into(), Prop::Bool(false));
            let sh = &el.style.shadow;
            let _ = write!(
                defs,
                "<filter id=\"{uid}-sh\" x=\"-30%\" y=\"-30%\" width=\"160%\" height=\"160%\"><feDropShadow dx=\"{}\" dy=\"{}\" stdDeviation=\"{:.1}\" flood-color=\"{}\" flood-opacity=\"{:.3}\"/></filter>",
                sh.x,
                sh.y,
                sh.blur / 2.0,
                sh.color.with_alpha(255).css(),
                sh.color.a as f32 / 255.0
            );
            let _ = write!(body, "<g filter=\"url(#{uid}-sh)\">");
        }
        let ch = chrome::chrome(el, &props, theme);
        body += &ch.scene.to_svg();
        let filter = String::new();
        match &el.kind {
            ElementKind::Shape(sp) => {
                out.kind = "shape".into();
                let st = shape_style(sp, &props, el);
                let fill = paint(&st.fill, &format!("{uid}-f"), &mut defs);
                let stroke = if st.stroke_w > 0.0 { format!(" stroke=\"{}\" stroke-width=\"{}\"{}", st.stroke_color.css(), st.stroke_w, dash_attr(&sp.stroke.dash, st.stroke_w)) } else { String::new() };
                let inset = st.stroke_w / 2.0;
                match outline(sp, w, h, st.radius) {
                    Outline::Rect(r) => {
                        let _ = write!(body, "<rect x=\"{inset:.2}\" y=\"{inset:.2}\" width=\"{:.2}\" height=\"{:.2}\" rx=\"{r:.1}\" fill=\"{fill}\"{stroke}{filter}/>", (w - 2.0 * inset).max(0.0), (h - 2.0 * inset).max(0.0));
                    }
                    Outline::Ellipse => {
                        let _ = write!(body, "<ellipse cx=\"{:.2}\" cy=\"{:.2}\" rx=\"{:.2}\" ry=\"{:.2}\" fill=\"{fill}\"{stroke}{filter}/>", w / 2.0, h / 2.0, (w / 2.0 - inset).max(0.0), (h / 2.0 - inset).max(0.0));
                    }
                    Outline::Line(a, b) => {
                        let col = if st.stroke_w > 0.0 { st.stroke_color.css() } else { st.fill.color().unwrap_or(theme.foreground).css() };
                        let sw = st.stroke_w.max(2.0);
                        let _ = write!(body, "<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"{col}\" stroke-width=\"{sw}\" stroke-linecap=\"round\"{}{filter}/>", a.0, a.1, b.0, b.1, dash_attr(&sp.stroke.dash, sw));
                    }
                    Outline::Poly(pts, closed) => {
                        if closed {
                            let _ = write!(body, "<polygon points=\"{}\" fill=\"{fill}\" stroke-linejoin=\"round\"{stroke}{filter}/>", points_attr(&pts));
                        } else {
                            let col = if st.stroke_w > 0.0 { st.stroke_color.css() } else { st.fill.color().unwrap_or(theme.foreground).css() };
                            let _ = write!(body, "<polyline points=\"{}\" fill=\"none\" stroke=\"{col}\" stroke-width=\"{}\" stroke-linejoin=\"round\" stroke-linecap=\"round\"{filter}/>", points_attr(&pts), st.stroke_w.max(2.0));
                        }
                    }
                }
                let text = shape_text(sp, &props);
                if !text.is_empty() {
                    let bx = Rect::new(8.0, 4.0, w - 16.0, h - 8.0);
                    let lines = layout_rich(&env, &text, theme, bx.w, bx.h, default_text_color(&st.fill, theme));
                    svg_text(&mut body, &lines, &text, bx);
                }
            }
            ElementKind::Text(t) => {
                out.kind = "text".into();
                let rich = text_rich(t, &props);
                let pad = el.style.padding.max(0.0).min(w / 4.0);
                let bx = Rect::new(pad, pad, (w - 2.0 * pad).max(1.0), (h - 2.0 * pad).max(1.0));
                let lines = layout_rich(&env, &rich, theme, bx.w, bx.h, theme.foreground);
                svg_text(&mut body, &lines, &rich, bx);
            }
            ElementKind::Image(img) => {
                out.kind = "image".into();
                let src = img_source(img, &props);
                let scaling = props.text("image.scaling", if img.scaling.is_empty() { "Fit" } else { &img.scaling }).to_lowercase();
                let par = match scaling.as_str() {
                    "fill" => "xMidYMid slice",
                    "normal" => "xMinYMin meet",
                    _ => "xMidYMid meet",
                };
                let r = el.style.radius.first().copied().unwrap_or(-1.0).max(0.0);
                if r > 0.0 {
                    let _ = write!(defs, "<clipPath id=\"{uid}-c\"><rect width=\"{w:.1}\" height=\"{h:.1}\" rx=\"{r}\"/></clipPath>");
                }
                let clip = if r > 0.0 { format!(" clip-path=\"url(#{uid}-c)\"") } else { String::new() };
                let href = match image_data(p, &src) {
                    Some((mime, b64)) => Some(format!("data:{mime};base64,{b64}")),
                    None if safe_url(&src) && !src.starts_with('#') => Some(esc(&src)),
                    None => None,
                };
                match href {
                    Some(href) => {
                        let _ = write!(body, "<image width=\"{w:.1}\" height=\"{h:.1}\" href=\"{href}\" preserveAspectRatio=\"{par}\"{clip}{filter}/>");
                    }
                    None => {
                        let _ = write!(
                            body,
                            "<rect width=\"{w:.1}\" height=\"{h:.1}\" rx=\"6\" fill=\"#EEF0F3\"/><text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"11\" fill=\"#8A93A0\">Image</text>",
                            w / 2.0,
                            h / 2.0 + 4.0
                        );
                    }
                }
                out.action = img.action.as_ref().and_then(|a| serde_json::to_value(a).ok());
            }
            ElementKind::Button(b) => {
                out.kind = "button".into();
                let bs = button_style(b, &props, el);
                let fill = paint(&bs.fill, &format!("{uid}-f"), &mut defs);
                let hover = paint(&bs.hover, &format!("{uid}-h"), &mut defs);
                let stroke = if el.style.stroke.width > 0.0 { format!(" stroke=\"{}\" stroke-width=\"{}\"", el.style.stroke.color.css(), el.style.stroke.width) } else { String::new() };
                let _ = write!(body, "<rect class=\"vc-btn-bg\" width=\"{w:.1}\" height=\"{h:.1}\" rx=\"{}\" fill=\"{fill}\"{stroke}{filter}/>", bs.radius);
                let _ = write!(body, "<rect class=\"vc-btn-hover\" width=\"{w:.1}\" height=\"{h:.1}\" rx=\"{}\" fill=\"{hover}\"/>", bs.radius);
                if let Some(nav) = &b.navigator {
                    navigator(&mut body, p, page, nav, w, h, bs.text_color, &bs.fill);
                } else {
                    let icon_w = button_icon(&mut body, &bs.icon, h, bs.text_color, !bs.text.is_empty(), w);
                    let bx = Rect::new(6.0 + icon_w, 2.0, (w - 12.0 - icon_w).max(1.0), (h - 4.0).max(1.0));
                    let lines = layout_rich(&env, &bs.text, theme, bx.w, bx.h, bs.text_color);
                    svg_text(&mut body, &lines, &bs.text, bx);
                    out.action = b.action.as_ref().and_then(|a| serde_json::to_value(a).ok()).or_else(|| match bs.icon.as_str() {
                        "Back" => serde_json::to_value(Action::Back).ok(),
                        "Reset" => serde_json::to_value(Action::ClearAllSlicers).ok(),
                        _ => None,
                    });
                }
            }
            ElementKind::Group | ElementKind::Visual(_) => {}
        }
        if el.style.shadow.enabled {
            body += "</g>";
        }
    }
    let opacity = if el.style.opacity < 1.0 && el.style.opacity >= 0.0 { format!(" opacity=\"{:.2}\"", el.style.opacity) } else { String::new() };
    let blend = if el.style.blend != "normal" && !el.style.blend.is_empty() { format!(" style=\"mix-blend-mode:{}\"", esc(&el.style.blend)) } else { String::new() };
    out.svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w:.2} {h:.2}\" width=\"100%\" height=\"100%\" overflow=\"visible\" font-family=\"{}\"{opacity}{blend}>{}{body}</svg>",
        FONT_STACK.replace('\'', "&apos;"),
        if defs.is_empty() { String::new() } else { format!("<defs>{defs}</defs>") }
    );
    out
}

/// A small glyph at the left of a button (or centred when the button has no text). Returns the
/// width it takes.
fn button_icon(out: &mut String, icon: &str, h: f32, c: Color, has_text: bool, w: f32) -> f32 {
    let s = (h * 0.42).clamp(10.0, 22.0);
    let cx = if has_text { 8.0 + s / 2.0 } else { w / 2.0 };
    let cy = h / 2.0;
    let col = c.css();
    let st = format!("fill=\"none\" stroke=\"{col}\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\"");
    let (x0, y0) = (cx - s / 2.0, cy - s / 2.0);
    let drawn = match icon {
        "Back" => {
            let _ = write!(out, "<path d=\"M{:.1},{cy:.1} H{:.1} M{:.1},{:.1} L{x0:.1},{cy:.1} L{:.1},{:.1}\" {st}/>", x0, x0 + s, x0 + s * 0.4, y0 + s * 0.15, x0 + s * 0.4, y0 + s * 0.85);
            true
        }
        "Arrow" | "Navigator" => {
            let _ = write!(out, "<path d=\"M{x0:.1},{cy:.1} H{:.1} M{:.1},{:.1} L{:.1},{cy:.1} L{:.1},{:.1}\" {st}/>", x0 + s, x0 + s * 0.6, y0 + s * 0.15, x0 + s, x0 + s * 0.6, y0 + s * 0.85);
            true
        }
        "Bookmark" => {
            let _ = write!(out, "<path d=\"M{:.1},{y0:.1} H{:.1} V{:.1} L{cx:.1},{:.1} L{:.1},{:.1} Z\" {st}/>", x0 + s * 0.2, x0 + s * 0.8, y0 + s, y0 + s * 0.72, x0 + s * 0.2, y0 + s);
            true
        }
        "Information" | "Info" => {
            let _ = write!(out, "<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{:.1}\" {st}/><path d=\"M{cx:.1},{:.1} V{:.1}\" {st}/><circle cx=\"{cx:.1}\" cy=\"{:.1}\" r=\"1.2\" fill=\"{col}\"/>", s / 2.0, cy - s * 0.05, cy + s * 0.25, cy - s * 0.25);
            true
        }
        "Help" | "Q&A" => {
            let _ = write!(out, "<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{:.1}\" {st}/><text x=\"{cx:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"{:.1}\" font-weight=\"700\" fill=\"{col}\">?</text>", s / 2.0, cy + s * 0.22, s * 0.62);
            true
        }
        "Reset" => {
            let _ = write!(out, "<path d=\"M{:.1},{:.1} A{:.1},{:.1} 0 1 1 {:.1},{:.1}\" {st}/><path d=\"M{:.1},{:.1} L{:.1},{:.1} L{:.1},{:.1}\" {st}/>", x0 + s * 0.15, cy, s * 0.42, s * 0.42, x0 + s * 0.3, y0 + s * 0.2, x0 + s * 0.05, y0 + s * 0.25, x0 + s * 0.15, cy, x0 + s * 0.42, cy - s * 0.05);
            true
        }
        _ => false,
    };
    if drawn && has_text { s + 6.0 } else { 0.0 }
}

/// A page navigator button: one tab per visible page (or bookmark).
#[allow(clippy::too_many_arguments)]
fn navigator(out: &mut String, p: &Project, page: &Page, nav: &str, w: f32, h: f32, text: Color, fill: &Fill) {
    let items: Vec<(String, String, bool)> = if nav.to_ascii_lowercase().starts_with("bookmark") {
        p.report.bookmarks.iter().map(|b| (b.name.clone(), serde_json::json!({"type": "bookmark", "id": b.id}).to_string(), false)).collect()
    } else {
        p.report
            .pages
            .iter()
            .filter(|pg| !pg.hidden && matches!(pg.kind, PageKind::Report | PageKind::Dashboard | PageKind::Worksheet))
            .map(|pg| (pg.name.clone(), serde_json::json!({"type": "pageNavigation", "page": pg.id}).to_string(), pg.id == page.id))
            .collect()
    };
    if items.is_empty() {
        return;
    }
    let gap = 4.0;
    let bw = (w - gap * (items.len() as f32 - 1.0)) / items.len() as f32;
    let accent = fill.color().unwrap_or(Color::rgb(0x2B, 0x6C, 0xE0));
    for (i, (name, act, on)) in items.iter().enumerate() {
        let x = i as f32 * (bw + gap);
        let (bg, fg) = if *on { (accent.shade(-0.25), text) } else { (Color::rgba(255, 255, 255, 0), text) };
        let _ = write!(
            out,
            "<g class=\"vc-act\" data-action=\"{}\"><rect x=\"{x:.1}\" y=\"0\" width=\"{bw:.1}\" height=\"{h:.1}\" rx=\"5\" fill=\"{}\"/><text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"{:.1}\" fill=\"{}\"{}>{}</text></g>",
            esc(act),
            bg.css(),
            x + bw / 2.0,
            h / 2.0 + 4.5,
            (h * 0.36).clamp(9.0, 14.0),
            fg.css(),
            if *on { " font-weight=\"600\"" } else { "" },
            esc(&vizardcraft_core::scene::ellipsize(name, (h * 0.36).clamp(9.0, 14.0), bw - 8.0))
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Layouts: desktop and phone
// ---------------------------------------------------------------------------------------------

/// Where an element goes on a canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub id: u64,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// The phone canvas width (points).
pub const PHONE_WIDTH: f32 = 360.0;

fn is_hidden(page: &Page, el: &Element) -> bool {
    if el.hidden || matches!(el.kind, ElementKind::Group) {
        return true;
    }
    // Hidden with its group.
    let mut parent = el.parent;
    let mut guard = 0;
    while let (Some(pid), true) = (parent, guard < 16) {
        match page.element(pid) {
            Some(g) if g.hidden => return true,
            Some(g) => parent = g.parent,
            None => break,
        }
        guard += 1;
    }
    false
}

/// The visible elements of a page in z-order, as placed on the desktop canvas.
pub fn desktop_layout(page: &Page) -> Vec<Placed> {
    page.elements.iter().filter(|e| !is_hidden(page, e)).map(|e| Placed { id: e.id, x: e.x, y: e.y, w: e.w, h: e.h }).collect()
}

/// The phone layout of a page: the user's mobile layout when there is one, otherwise a
/// generated single column (reading order; cards in pairs; banners keep the elements on them).
/// Returns the placements and the canvas height.
pub fn mobile_layout(page: &Page) -> (Vec<Placed>, f32) {
    if !page.mobile.items.is_empty() {
        let items: Vec<Placed> = page
            .mobile
            .items
            .iter()
            .filter(|m| page.element(m.element).is_some_and(|e| !is_hidden(page, e)))
            .map(|m| Placed { id: m.element, x: m.x, y: m.y, w: m.w, h: m.h })
            .collect();
        let hgt = items.iter().map(|m| m.y + m.h).fold(0.0, f32::max) + 8.0;
        return (items, hgt.max(200.0));
    }
    let width = if page.mobile.width > 200.0 { page.mobile.width } else { PHONE_WIDTH };
    let pad = 8.0;
    let gap = 8.0;
    let inner = width - 2.0 * pad;
    let els: Vec<&Element> = page.elements.iter().filter(|e| !is_hidden(page, e)).collect();
    let contains = |a: &Element, b: &Element| a.id != b.id && b.x >= a.x - 1.0 && b.y >= a.y - 1.0 && b.x + b.w <= a.x + a.w + 1.0 && b.y + b.h <= a.y + a.h + 1.0;
    let is_container = |e: &Element| matches!(e.kind, ElementKind::Shape(_)) && els.iter().any(|o| contains(e, o));
    // Elements on a banner/background shape travel with it.
    let owner = |e: &Element| -> Option<u64> { els.iter().filter(|c| is_container(c) && contains(c, e)).min_by(|a, b| (a.w * a.h).total_cmp(&(b.w * b.h))).map(|c| c.id) };
    let decorative = |e: &Element| match &e.kind {
        ElementKind::Shape(s) => matches!(s.shape, ShapeKind::Line) || (s.text.is_empty() && !is_container(e)),
        _ => false,
    };
    let mut top: Vec<&Element> = els.iter().copied().filter(|e| owner(e).is_none() && !decorative(e)).collect();
    // Reading order: rows (by vertical overlap), then left to right.
    top.sort_by(|a, b| {
        let row = |e: &Element| (e.y / 24.0).round() as i64;
        row(a).cmp(&row(b)).then(a.x.total_cmp(&b.x))
    });
    let small = |e: &Element| matches!(e.visual().map(|v| v.kind), Some(VisualKind::Card | VisualKind::Kpi | VisualKind::Gauge)) && e.w <= 340.0;
    let mut out = Vec::new();
    let mut y = pad;
    let mut i = 0;
    while let Some(e) = top.get(i).copied() {
        if small(e) {
            // Two cards per row.
            let next = top.get(i + 1).copied().filter(|n| small(n));
            let cw = (inner - gap) / 2.0;
            let h = e.h.clamp(88.0, 130.0);
            out.push(Placed { id: e.id, x: pad, y, w: cw, h });
            if let Some(n) = next {
                out.push(Placed { id: n.id, x: pad + cw + gap, y, w: cw, h });
                i += 1;
            } else if let Some(last) = out.last_mut() {
                last.w = inner;
            }
            y += h + gap;
            i += 1;
            continue;
        }
        let h = match &e.kind {
            ElementKind::Visual(v) => match v.kind {
                VisualKind::Slicer => e.h.clamp(64.0, 220.0),
                VisualKind::Table | VisualKind::Matrix => e.h.clamp(200.0, 420.0),
                VisualKind::MultiRowCard => e.h.clamp(120.0, 300.0),
                _ => (e.h * 0.92).clamp(220.0, 340.0),
            },
            ElementKind::Text(_) => e.h.clamp(28.0, 160.0) * (e.w / inner).clamp(1.0, 2.5).sqrt(),
            ElementKind::Shape(_) => e.h.clamp(48.0, 160.0),
            ElementKind::Image(_) => (e.h * inner / e.w.max(1.0)).clamp(40.0, 240.0),
            ElementKind::Button(_) => e.h.clamp(32.0, 56.0),
            ElementKind::Group => 0.0,
        };
        let full_bleed = is_container(e) && e.y < 4.0 && e.w > page.width * 0.8;
        let (x, w) = if full_bleed { (0.0, width) } else { (pad, inner) };
        let (yy, hh) = if full_bleed && out.is_empty() { (0.0, h + pad) } else { (y, h) };
        out.push(Placed { id: e.id, x, y: yy, w, h: hh });
        if is_container(e) {
            // Children keep their offset from the container's left/top, clipped to its width.
            for c in els.iter().filter(|c| owner(c) == Some(e.id)) {
                let ox = (c.x - e.x).min(w * 0.25);
                let oy = (c.y - e.y) + if full_bleed && out.len() == 1 { pad / 2.0 } else { 0.0 };
                let cw = (c.w).min(w - ox - pad);
                let ch = (c.h).min(hh - oy).max(16.0);
                out.push(Placed { id: c.id, x: x + ox, y: yy + oy, w: cw.max(20.0), h: ch });
            }
        }
        y = yy + hh + gap;
        i += 1;
    }
    (out, y + pad)
}

/// Renders an element at a placement (another size and position than its own).
pub fn element_at(p: &Project, built: &Built, rt: &Runtime, page: &Page, el: &Element, at: &Placed, uid: &str) -> ElementOut {
    if (at.w - el.w).abs() < 0.01 && (at.h - el.h).abs() < 0.01 {
        let mut o = element_svg(p, built, rt, page, el, uid);
        o.x = at.x;
        o.y = at.y;
        return o;
    }
    let mut e2 = el.clone();
    e2.x = at.x;
    e2.y = at.y;
    e2.w = at.w;
    e2.h = at.h;
    element_svg(p, built, rt, page, &e2, uid)
}

/// A whole page as one SVG document (thumbnails, `render --svg`, SVG export).
pub fn page_svg(p: &Project, built: &Built, rt: &Runtime, page: &Page, mobile: bool) -> String {
    let (placed, w, h) = if mobile {
        let (pl, h) = mobile_layout(page);
        (pl, PHONE_WIDTH, h)
    } else {
        (desktop_layout(page), page.width, page.height)
    };
    let mut defs = String::new();
    let bg = paint(&page.background, "vc-page-bg", &mut defs);
    let mut body = String::new();
    for pl in &placed {
        let Some(el) = page.element(pl.id) else { continue };
        let o = element_at(p, built, rt, page, el, pl, &format!("e{}", el.id));
        let rot = if o.rot != 0.0 { format!(" transform=\"rotate({} {:.1} {:.1})\"", o.rot, o.x + o.w / 2.0, o.y + o.h / 2.0) } else { String::new() };
        let inner = o.svg.replacen("width=\"100%\" height=\"100%\"", &format!("x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\"", o.x, o.y, o.w, o.h), 1);
        if rot.is_empty() {
            body += &inner;
        } else {
            let _ = write!(body, "<g{rot}>{inner}</g>");
        }
    }
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\" font-family=\"{}\">{}<rect width=\"{w}\" height=\"{h}\" fill=\"{bg}\"/>{body}</svg>",
        FONT_STACK.replace('\'', "&apos;"),
        if defs.is_empty() { String::new() } else { format!("<defs>{defs}</defs>") }
    )
}
