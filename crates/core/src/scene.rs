//! A renderer-independent drawing of a visual: what [`crate::charts`] produce, the desktop
//! paints with egui and the web viewer writes as SVG ([`Scene::to_svg`]). Coordinates are in
//! points with the origin at the top-left of the visual's content box.

use crate::color::Color;
use crate::doc::FieldRef;
use crate::scalar::Scalar;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.right() && y >= self.y && y <= self.bottom()
    }
    pub fn shrink(&self, d: f32) -> Rect {
        Rect::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub width: f32,
    pub color: Color,
    /// Dash pattern (on, off) in points; None = solid.
    pub dash: Option<(f32, f32)>,
}

impl Line {
    pub fn solid(width: f32, color: Color) -> Line {
        Line { width, color, dash: None }
    }
    pub fn dashed(width: f32, color: Color) -> Line {
        Line { width, color, dash: Some((4.0, 3.0)) }
    }
    pub fn from_style(width: f32, color: Color, style: &str) -> Line {
        let dash = match style {
            "dash" | "dashed" => Some((5.0, 3.0)),
            "dot" | "dotted" => Some((1.5, 2.5)),
            _ => None,
        };
        Line { width, color, dash }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Rect { rect: Rect, fill: Option<Color>, stroke: Option<Line>, radius: f32 },
    /// Polyline or polygon.
    Path { points: Vec<(f32, f32)>, closed: bool, fill: Option<Color>, stroke: Option<Line> },
    Circle { cx: f32, cy: f32, r: f32, fill: Option<Color>, stroke: Option<Line> },
    /// Annular sector (pie/donut slices, gauges): angles in radians, 0 = 12 o'clock, clockwise.
    Arc { cx: f32, cy: f32, r_inner: f32, r_outer: f32, start: f32, end: f32, fill: Color, stroke: Option<Line> },
    Text { x: f32, y: f32, text: String, size: f32, color: Color, bold: bool, italic: bool, h: Align, v: Align, angle: f32 },
    /// An image (base64 PNG) placed in a rectangle: script visuals.
    Image { rect: Rect, png_base64: String },
}

/// What a click on an item selects (cross-filter / drill identity) and what its tooltip says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hit {
    pub point: Vec<(FieldRef, Scalar)>,
    pub tooltip: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub shape: Shape,
    pub hit: Option<Hit>,
    /// Dimmed (not highlighted) under cross-highlight or selection.
    pub dimmed: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub width: f32,
    pub height: f32,
    pub items: Vec<Item>,
    /// A message instead of a chart ("Add data fields…", errors).
    pub message: Option<String>,
    /// Interactive regions the renderer should offer (e.g. matrix expand buttons, slicer items):
    /// (rect, action id).
    pub actions: Vec<(Rect, String)>,
}

impl Scene {
    pub fn new(width: f32, height: f32) -> Scene {
        Scene { width, height, ..Scene::default() }
    }
    pub fn message(width: f32, height: f32, msg: impl Into<String>) -> Scene {
        Scene { width, height, message: Some(msg.into()), ..Scene::default() }
    }
    pub fn push(&mut self, shape: Shape) {
        self.items.push(Item { shape, hit: None, dimmed: false });
    }
    pub fn push_hit(&mut self, shape: Shape, hit: Hit, dimmed: bool) {
        self.items.push(Item { shape, hit: Some(hit), dimmed });
    }
    pub fn rect(&mut self, rect: Rect, fill: Color) {
        self.push(Shape::Rect { rect, fill: Some(fill), stroke: None, radius: 0.0 });
    }
    pub fn line(&mut self, a: (f32, f32), b: (f32, f32), stroke: Line) {
        self.push(Shape::Path { points: vec![a, b], closed: false, fill: None, stroke: Some(stroke) });
    }
    #[allow(clippy::too_many_arguments)]
    pub fn text(&mut self, x: f32, y: f32, text: impl Into<String>, size: f32, color: Color, h: Align, v: Align) {
        self.push(Shape::Text { x, y, text: text.into(), size, color, bold: false, italic: false, h, v, angle: 0.0 });
    }
    pub fn text_bold(&mut self, x: f32, y: f32, text: impl Into<String>, size: f32, color: Color, h: Align, v: Align) {
        self.push(Shape::Text { x, y, text: text.into(), size, color, bold: true, italic: false, h, v, angle: 0.0 });
    }
    /// The topmost hit item at a point.
    pub fn hit_at(&self, x: f32, y: f32) -> Option<&Hit> {
        self.items.iter().rev().find(|it| it.hit.is_some() && shape_contains(&it.shape, x, y)).and_then(|it| it.hit.as_ref())
    }
    pub fn action_at(&self, x: f32, y: f32) -> Option<&str> {
        self.actions.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, a)| a.as_str())
    }

    /// The scene as an SVG fragment (`<g>…</g>`), for the web viewer and SVG/HTML export.
    /// Hit items carry `data-hit` (JSON identity) and `data-tip` attributes.
    pub fn to_svg(&self) -> String {
        crate::charts::svg::scene_svg(self)
    }
}

/// Approximate text width (points) for layout: average glyph width ≈ 0.55 em.
pub fn text_width(text: &str, size: f32) -> f32 {
    text.chars().map(|c| if c.is_ascii_uppercase() || c == 'm' || c == 'w' { 0.68 } else if "il.,:;|!'".contains(c) { 0.3 } else { 0.55 }).sum::<f32>() * size
}

/// Shortens text to fit a width, with an ellipsis.
pub fn ellipsize(text: &str, size: f32, max_w: f32) -> String {
    if text_width(text, size) <= max_w {
        return text.to_string();
    }
    let mut s = String::new();
    for c in text.chars() {
        if text_width(&format!("{s}{c}…"), size) > max_w {
            break;
        }
        s.push(c);
    }
    format!("{s}…")
}

pub fn shape_contains(s: &Shape, x: f32, y: f32) -> bool {
    match s {
        Shape::Rect { rect, .. } | Shape::Image { rect, .. } => rect.contains(x, y),
        Shape::Circle { cx, cy, r, .. } => {
            let rr = r.max(4.0);
            (x - cx).powi(2) + (y - cy).powi(2) <= rr * rr
        }
        Shape::Arc { cx, cy, r_inner, r_outer, start, end, .. } => {
            let dx = x - cx;
            let dy = y - cy;
            let d = (dx * dx + dy * dy).sqrt();
            if d < *r_inner || d > *r_outer {
                return false;
            }
            // Angle from 12 o'clock, clockwise.
            let mut a = dx.atan2(-dy);
            if a < 0.0 {
                a += std::f32::consts::TAU;
            }
            a >= *start && a <= *end
        }
        Shape::Path { points, closed, stroke, .. } => {
            if *closed && points.len() > 2 {
                point_in_polygon(points, x, y)
            } else {
                let tol = stroke.as_ref().map(|s| s.width).unwrap_or(1.0).max(5.0);
                points.windows(2).any(|w| match w {
                    [a, b] => seg_dist(*a, *b, (x, y)) <= tol,
                    _ => false,
                })
            }
        }
        Shape::Text { x: tx, y: ty, text, size, h, v, .. } => {
            let w = text_width(text, *size);
            let left = match h {
                Align::Start => *tx,
                Align::Center => tx - w / 2.0,
                Align::End => tx - w,
            };
            let top = match v {
                Align::Start => *ty,
                Align::Center => ty - size / 2.0,
                Align::End => ty - size,
            };
            Rect::new(left, top, w, *size * 1.2).contains(x, y)
        }
    }
}

fn point_in_polygon(pts: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut inside = false;
    let n = pts.len();
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = pts.get(i).copied().unwrap_or_default();
        let (xj, yj) = pts.get(j).copied().unwrap_or_default();
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi + f32::EPSILON) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn seg_dist(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let (qx, qy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - qx).powi(2) + (p.1 - qy).powi(2)).sqrt()
}
