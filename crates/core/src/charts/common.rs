//! Shared chart helpers: number formatting, colours, selection dimming, tooltips, label
//! templates, curve interpolation, markers and scene composition.

use super::RenderOpts;
use crate::color::Color;
use crate::doc::*;
use crate::scalar::Scalar;
use crate::scene::{Align, Hit, Line, Rect, Scene, Shape, ellipsize, text_width};
use crate::vquery::VisualData;

/// A "nice" axis: ticks covering [lo, hi].
pub fn nice_ticks(lo: f64, hi: f64, target: usize) -> Vec<f64> {
    if !lo.is_finite() || !hi.is_finite() {
        return vec![0.0, 1.0];
    }
    let (lo, hi) = if (hi - lo).abs() < 1e-12 {
        if lo == 0.0 { (0.0, 1.0) } else { (lo - lo.abs() * 0.5, hi + hi.abs() * 0.5) }
    } else {
        (lo.min(hi), lo.max(hi))
    };
    let step = nice_step((hi - lo) / target.max(1) as f64);
    let start = (lo / step).floor() * step;
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        let v = start + step * i as f64;
        let v = (v / step).round() * step;
        out.push(if v.abs() < step * 1e-9 { 0.0 } else { v });
        if v >= hi - step * 1e-9 || out.len() >= 60 {
            break;
        }
        i += 1;
    }
    out
}

pub fn nice_step(raw: f64) -> f64 {
    if !(raw.is_finite() && raw > 0.0) {
        return 1.0;
    }
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    mag * if norm <= 1.0 {
        1.0
    } else if norm <= 2.0 {
        2.0
    } else if norm <= 2.5 {
        2.5
    } else if norm <= 5.0 {
        5.0
    } else {
        10.0
    }
}

/// Compact number with display units ("Auto" picks K/M/bn).
pub fn short_num(v: f64, units: &str) -> String {
    if !v.is_finite() {
        return String::new();
    }
    let trim = |s: String| -> String {
        if s.contains('.') {
            let t = s.trim_end_matches('0').trim_end_matches('.');
            t.to_string()
        } else {
            s
        }
    };
    let (div, suf) = match units {
        "None" => return crate::numfmt::format(v, if v.fract().abs() > 1e-9 && v.abs() < 100.0 { "#,0.##" } else { "#,0" }),
        "Thousands" => (1e3, "K"),
        "Millions" => (1e6, "M"),
        "Billions" => (1e9, "bn"),
        "Trillions" => (1e12, "T"),
        _ => {
            let a = v.abs();
            if a >= 1e12 {
                (1e12, "T")
            } else if a >= 1e9 {
                (1e9, "bn")
            } else if a >= 1e6 {
                (1e6, "M")
            } else if a >= 1e3 {
                (1e3, "K")
            } else {
                (1.0, "")
            }
        }
    };
    let x = v / div;
    let s = if x.abs() >= 100.0 { format!("{x:.0}") } else if x.abs() >= 10.0 { trim(format!("{x:.1}")) } else { trim(format!("{x:.2}")) };
    format!("{s}{suf}")
}

/// How values of a column are written on axes and labels.
#[derive(Clone, Debug, Default)]
pub struct ValueFmt {
    /// The column's format string (`$#,0`, `0.0%`).
    pub format: String,
    /// Display units: "Auto", "None", "Thousands", …
    pub units: String,
    /// Decimal places (-1 = automatic).
    pub decimals: i32,
    /// An explicit format string from the format pane (wins).
    pub explicit: String,
}

impl ValueFmt {
    pub fn new(format: &str) -> ValueFmt {
        ValueFmt { format: format.to_string(), units: "Auto".into(), decimals: -1, explicit: String::new() }
    }
    pub fn of_col(d: &VisualData, col: usize) -> ValueFmt {
        ValueFmt::new(&d.cols.get(col).map(|c| c.format.clone()).unwrap_or_default())
    }
    pub fn with_props(mut self, o: &RenderOpts, prefix: &str) -> ValueFmt {
        self.units = o.props.text(&format!("{prefix}.units"), "Auto");
        self.decimals = o.props.num(&format!("{prefix}.decimals"), -1.0) as i32;
        self.explicit = o.props.text(&format!("{prefix}.format"), "");
        self
    }
    /// Picks one display unit for a set of values (so labels read consistently).
    pub fn uniform(mut self, max_abs: f64) -> ValueFmt {
        if self.units == "Auto" && !self.is_percent() {
            self.units = if max_abs >= 1e9 {
                "Billions"
            } else if max_abs >= 1e6 {
                "Millions"
            } else if max_abs >= 1e4 {
                "Thousands"
            } else {
                "None"
            }
            .into();
        }
        self
    }
    pub fn is_percent(&self) -> bool {
        self.format.contains('%')
    }
    fn currency(&self) -> &'static str {
        for (sym, s) in [("$", "$"), ("€", "€"), ("£", "£"), ("₹", "₹"), ("¥", "¥")] {
            if self.format.contains(sym) {
                return s;
            }
        }
        ""
    }
    /// Full-precision text (tooltips, tables).
    pub fn full(&self, v: f64) -> String {
        if !v.is_finite() {
            return String::new();
        }
        if !self.explicit.is_empty() {
            return crate::numfmt::format(v, &self.explicit);
        }
        crate::vquery::format_value(v, &self.format, crate::build::ColType::Number)
    }
    /// Compact text for labels and axis ticks.
    pub fn short(&self, v: f64) -> String {
        if !v.is_finite() {
            return String::new();
        }
        if !self.explicit.is_empty() {
            return crate::numfmt::format(v, &self.explicit);
        }
        if self.is_percent() {
            let dec = if self.decimals >= 0 { self.decimals as usize } else if (v * 100.0).fract().abs() < 1e-6 || v.abs() >= 0.1 { 0 } else { 1 };
            let dec = if self.decimals < 0 && (v * 100.0 - (v * 100.0).round()).abs() > 0.05 { dec.max(1) } else { dec };
            return format!("{:.*}%", dec, v * 100.0);
        }
        let cur = self.currency();
        let neg = v < 0.0;
        let a = v.abs();
        let body = if self.decimals >= 0 && (self.units == "None" || self.units == "Auto" && a < 1e4) {
            crate::numfmt::format(a, &format!("#,0{}", if self.decimals > 0 { format!(".{}", "0".repeat(self.decimals as usize)) } else { String::new() }))
        } else if self.units == "Auto" && a < 1e4 {
            if a.fract().abs() > 1e-9 && a < 100.0 { crate::numfmt::format(a, "#,0.##") } else { crate::numfmt::format(a, "#,0") }
        } else {
            short_num(a, &self.units)
        };
        format!("{}{cur}{body}", if neg { "-" } else { "" })
    }
}

/// The colour of series `i`, honoring per-point overrides and the default colour.
pub fn series_color(o: &RenderOpts, i: usize, key: &str) -> Color {
    if let Some(c) = o.props.point_colors.get(key) {
        return *c;
    }
    if i == 0
        && let Some(c) = o.props.color_opt("dataColors.default")
    {
        return c;
    }
    o.theme.series_color(i)
}

/// Applies the "dataColors.transparency" property.
pub fn data_alpha(o: &RenderOpts, c: Color) -> Color {
    let t = o.props.f32("dataColors.transparency", 0.0).clamp(0.0, 100.0);
    if t > 0.0 { c.with_alpha(((100.0 - t) / 100.0 * c.a as f32) as u8) } else { c }
}

fn is_selected(o: &RenderOpts, point: &[(FieldRef, Scalar)]) -> bool {
    o.selected.iter().any(|p| p.as_slice() == point || (!p.is_empty() && p.iter().all(|x| point.contains(x))))
}

/// Dimmed when something else is selected in this visual.
pub fn dim(o: &RenderOpts, point: &[(FieldRef, Scalar)]) -> bool {
    !o.selected.is_empty() && !is_selected(o, point)
}

pub fn tooltip(d: &VisualData, row: usize) -> Vec<(String, String)> {
    d.cols.iter().enumerate().filter(|(_, c)| c.role != "__internal").map(|(i, c)| (c.label.clone(), d.text(row, i))).collect()
}

/// Identity roles used for cross-filtering from cartesian charts.
pub const POINT_ROLES: &[&str] = &["axis", "legend", "small", "details", "rows", "columns", "location", "breakdown", "source", "destination"];
/// Roles fx data colours are keyed by (see `fx::resolve_point_colors`).
pub const COLOR_ROLES: &[&str] = &["axis", "legend", "details", "rows", "location"];

pub fn hit(d: &VisualData, row: usize, roles: &[&str]) -> Hit {
    Hit { point: d.identity(row, roles), tooltip: tooltip(d, row) }
}

/// The per-point colour override key of a row.
pub fn color_key(d: &VisualData, row: usize) -> String {
    crate::fx::point_key(&d.identity(row, COLOR_ROLES))
}

/// The value text of the highlight data (cross-highlight) for a cell.
pub fn hl_num(d: &VisualData, row: usize, col: usize) -> Option<f64> {
    d.highlight.as_ref()?.get(row)?.get(col)?.as_f64()
}

/// Expands a data-label template: `{value} {category} {series} {percent}`.
pub fn label_template(o: &RenderOpts, value: &str, category: &str, series: &str, percent: &str) -> String {
    let t = o.props.text("labels.template", "");
    if t.trim().is_empty() {
        return value.to_string();
    }
    t.replace("{value}", value).replace("{category}", category).replace("{series}", series).replace("{percent}", percent)
}

/// Gridline stroke from the format pane.
pub fn grid_line(o: &RenderOpts) -> Line {
    let c = grid_color(o);
    Line::from_style(o.props.f32("grid.width", 1.0), c, &o.props.text("grid.style", "solid"))
}

/// The gridline colour: the format pane's, except that its light default gives way to a
/// theme-derived colour on dark themes.
pub fn grid_color(o: &RenderOpts) -> Color {
    match o.props.color_opt("grid.color") {
        Some(c) if !(c == Color::rgb(0xE6, 0xE8, 0xEC) && o.theme.visual_background.luminance() < 0.4) => c,
        _ => grid_default(o),
    }
}

pub fn grid_default(o: &RenderOpts) -> Color {
    if o.theme.visual_background.luminance() < 0.4 { o.theme.visual_border } else { Color::rgb(0xE9, 0xEC, 0xF0) }
}

/// Muted axis label colour.
pub fn axis_text(o: &RenderOpts, prefix: &str) -> Color {
    o.props.color_opt(&format!("{prefix}.color")).unwrap_or(o.theme.foreground_muted)
}

/// Text on top of a coloured mark.
pub fn label_color(o: &RenderOpts, on: Option<Color>) -> Color {
    if let Some(c) = o.props.color_opt("labels.color") {
        return c;
    }
    match on {
        Some(c) => c.contrast_text(),
        None => o.theme.foreground,
    }
}

/// Catmull-Rom smoothing (clamped so curves don't overshoot much), sampled into a polyline.
pub fn smooth(pts: &[(f32, f32)]) -> Vec<(f32, f32)> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut out = Vec::with_capacity(pts.len() * 8);
    let get = |i: isize| -> (f32, f32) {
        let i = i.clamp(0, pts.len() as isize - 1) as usize;
        pts.get(i).copied().unwrap_or_default()
    };
    for i in 0..pts.len() - 1 {
        let p0 = get(i as isize - 1);
        let p1 = get(i as isize);
        let p2 = get(i as isize + 1);
        let p3 = get(i as isize + 2);
        let (ylo, yhi) = (p1.1.min(p2.1), p1.1.max(p2.1));
        let steps = 10;
        for s in 0..steps {
            let t = s as f32 / steps as f32;
            let t2 = t * t;
            let t3 = t2 * t;
            let x = 0.5 * ((2.0 * p1.0) + (-p0.0 + p2.0) * t + (2.0 * p0.0 - 5.0 * p1.0 + 4.0 * p2.0 - p3.0) * t2 + (-p0.0 + 3.0 * p1.0 - 3.0 * p2.0 + p3.0) * t3);
            let y = 0.5 * ((2.0 * p1.1) + (-p0.1 + p2.1) * t + (2.0 * p0.1 - 5.0 * p1.1 + 4.0 * p2.1 - p3.1) * t2 + (-p0.1 + 3.0 * p1.1 - 3.0 * p2.1 + p3.1) * t3);
            // Keep within the segment's y-range (monotone-ish).
            let pad = (yhi - ylo) * 0.15;
            out.push((x, y.clamp(ylo - pad, yhi + pad)));
        }
    }
    if let Some(l) = pts.last() {
        out.push(*l);
    }
    out
}

/// Step interpolation (horizontal then vertical, centred on points).
pub fn step(pts: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let mut out = Vec::with_capacity(pts.len() * 2);
    for (i, p) in pts.iter().enumerate() {
        if i == 0 {
            out.push(*p);
            continue;
        }
        let prev = pts.get(i - 1).copied().unwrap_or(*p);
        let mx = (prev.0 + p.0) / 2.0;
        out.push((mx, prev.1));
        out.push((mx, p.1));
        out.push(*p);
    }
    out
}

pub fn interpolate(o: &RenderOpts, pts: &[(f32, f32)]) -> Vec<(f32, f32)> {
    match o.props.text("lines.interpolation", "Linear").as_str() {
        "Smooth" => smooth(pts),
        "Step" => step(pts),
        _ => pts.to_vec(),
    }
}

/// A marker shape at a point.
pub fn marker_shape(shape: &str, x: f32, y: f32, r: f32, fill: Color, ring: Option<Color>) -> Shape {
    let stroke = ring.map(|c| Line::solid(1.5, c));
    match shape {
        "Square" => Shape::Rect { rect: Rect::new(x - r, y - r, r * 2.0, r * 2.0), fill: Some(fill), stroke, radius: 1.0 },
        "Diamond" => Shape::Path { points: vec![(x, y - r * 1.25), (x + r * 1.25, y), (x, y + r * 1.25), (x - r * 1.25, y)], closed: true, fill: Some(fill), stroke },
        "Triangle" => Shape::Path { points: vec![(x, y - r * 1.25), (x + r * 1.15, y + r * 0.9), (x - r * 1.15, y + r * 0.9)], closed: true, fill: Some(fill), stroke },
        _ => Shape::Circle { cx: x, cy: y, r, fill: Some(fill), stroke },
    }
}

/// Copies `src` into `dst`, offset by (dx, dy) (small multiples, Q&A, composite visuals).
pub fn embed(dst: &mut Scene, src: Scene, dx: f32, dy: f32) {
    for mut it in src.items {
        offset_shape(&mut it.shape, dx, dy);
        dst.items.push(it);
    }
    for (r, a) in src.actions {
        dst.actions.push((Rect::new(r.x + dx, r.y + dy, r.w, r.h), a));
    }
    if let Some(m) = src.message {
        dst.text(dx + src.width / 2.0, dy + src.height / 2.0, m, 11.0, Color::rgb(0x88, 0x88, 0x88), Align::Center, Align::Center);
    }
}

pub fn offset_shape(s: &mut Shape, dx: f32, dy: f32) {
    match s {
        Shape::Rect { rect, .. } | Shape::Image { rect, .. } => {
            rect.x += dx;
            rect.y += dy;
        }
        Shape::Path { points, .. } => {
            for p in points.iter_mut() {
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

/// Text with a soft rounded background (data labels with "labels.background").
#[allow(clippy::too_many_arguments)]
pub fn label_box(s: &mut Scene, o: &RenderOpts, x: f32, y: f32, text: &str, size: f32, color: Color, h: Align, v: Align) {
    if o.props.bool("labels.background", false) {
        let w = text_width(text, size) + 6.0;
        let left = match h {
            Align::Start => x - 3.0,
            Align::Center => x - w / 2.0,
            Align::End => x - w + 3.0,
        };
        let top = match v {
            Align::Start => y - 1.0,
            Align::Center => y - size * 0.65,
            Align::End => y - size * 1.2,
        };
        s.push(Shape::Rect { rect: Rect::new(left, top, w, size * 1.35), fill: Some(o.theme.visual_background.with_alpha(220)), stroke: None, radius: 3.0 });
    }
    s.text(x, y, text, size, color, h, v);
}

/// A rounded rectangle helper.
pub fn rrect(rect: Rect, fill: Color, radius: f32) -> Shape {
    Shape::Rect { rect, fill: Some(fill), stroke: None, radius }
}

/// Wraps text into lines that fit `max_w`.
pub fn wrap(text: &str, size: f32, max_w: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut cur = String::new();
        for w in para.split_whitespace() {
            let cand = if cur.is_empty() { w.to_string() } else { format!("{cur} {w}") };
            if text_width(&cand, size) > max_w && !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
                cur = w.to_string();
            } else {
                cur = cand;
            }
        }
        lines.push(cur);
    }
    lines
}

/// The display label of a category value (dates formatted with the column's format).
pub fn cat_text(d: &VisualData, row: usize, col: usize) -> String {
    match d.cell(row, col) {
        Scalar::Null => "(Blank)".into(),
        // Whole numbers in an unformatted category (years, ids): no thousands separator.
        Scalar::Num(n) if n.fract() == 0.0 && d.cols.get(col).is_some_and(|c| c.format.is_empty() && !c.col_type.is_temporal()) => crate::numfmt::general(*n),
        _ => d.text(row, col),
    }
}

/// "January" → "Jan" (for crowded month axes).
pub fn abbreviate_months(label: &str) -> String {
    const M: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let mut out = label.to_string();
    for m in M {
        if out.contains(m) {
            out = out.replace(m, m.get(..3).unwrap_or(m));
        }
    }
    out
}

/// A short, centred placeholder for empty visuals.
pub fn empty(o: &RenderOpts, msg: &str) -> Scene {
    super::message(o, msg)
}

pub fn ell(text: &str, size: f32, w: f32) -> String {
    ellipsize(text, size, w.max(8.0))
}

/// The default gradient (low → high) for heat maps / filled maps: one hue, light to dark.
pub fn seq_color(o: &RenderOpts, t: f64) -> Color {
    let hi = o.props.color_opt("dataColors.max").or(o.props.color_opt("dataColors.default")).unwrap_or(o.theme.series_color(0));
    let lo = o.props.color_opt("dataColors.min").unwrap_or(o.theme.visual_background.lerp(hi, 0.08));
    lo.lerp(hi, t.clamp(0.0, 1.0))
}
