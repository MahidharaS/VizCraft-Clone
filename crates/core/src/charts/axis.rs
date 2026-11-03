//! Axes (value scales with nice ticks, log/invert/min/max; category label layout with
//! thinning and rotation) and legends.

use super::RenderOpts;
use super::common::*;
use crate::color::Color;
use crate::scene::{Align, Hit, Line, Rect, Scene, Shape, text_width};

/// A numeric scale.
#[derive(Clone, Debug)]
pub struct Scale {
    pub lo: f64,
    pub hi: f64,
    pub ticks: Vec<f64>,
    pub log: bool,
    pub invert: bool,
}

impl Scale {
    /// A linear scale over [min, max] with nice ticks.
    pub fn linear(min: f64, max: f64, include_zero: bool, target: usize) -> Scale {
        let (mut a, mut b) = if min.is_finite() && max.is_finite() { (min.min(max), min.max(max)) } else { (0.0, 1.0) };
        if include_zero {
            a = a.min(0.0);
            b = b.max(0.0);
        }
        let ticks = nice_ticks(a, b, target);
        let lo = ticks.first().copied().unwrap_or(a);
        let hi = ticks.last().copied().unwrap_or(b).max(lo + 1e-9);
        Scale { lo, hi, ticks, log: false, invert: false }
    }

    /// A scale honoring the axis properties `<prefix>.min/max/scale/invert`.
    pub fn from_props(o: &RenderOpts, prefix: &str, min: f64, max: f64, include_zero: bool, target: usize) -> Scale {
        let pmin = o.props.text(&format!("{prefix}.min"), "").trim().parse::<f64>().ok();
        let pmax = o.props.text(&format!("{prefix}.max"), "").trim().parse::<f64>().ok();
        let log = o.props.text(&format!("{prefix}.scale"), "Linear") == "Log";
        let invert = o.props.bool(&format!("{prefix}.invert"), false);
        let mut s = if log {
            let lo_d = if min > 0.0 { min } else { 1.0 };
            let hi_d = max.max(lo_d * 10.0);
            let lo = pmin.filter(|v| *v > 0.0).unwrap_or(10f64.powf(lo_d.log10().floor()));
            let hi = pmax.filter(|v| *v > lo).unwrap_or(10f64.powf(hi_d.log10().ceil()));
            let mut ticks = Vec::new();
            let mut t = 10f64.powf(lo.log10().ceil());
            while t <= hi * 1.0001 && ticks.len() < 20 {
                ticks.push(t);
                t *= 10.0;
            }
            Scale { lo, hi, ticks, log: true, invert }
        } else {
            let mut s = Scale::linear(pmin.unwrap_or(min), pmax.unwrap_or(max), include_zero && pmin.is_none(), target);
            if let Some(a) = pmin {
                s.lo = a;
                s.ticks.retain(|t| *t >= a - 1e-9);
            }
            if let Some(b) = pmax {
                s.hi = b.max(s.lo + 1e-9);
                s.ticks.retain(|t| *t <= b + 1e-9);
            }
            s
        };
        s.invert = invert;
        s
    }

    /// Position of a value as a fraction of the axis length (0 = start, clamped to [-0.05, 1.05]).
    pub fn frac(&self, v: f64) -> f32 {
        let t = if self.log {
            let v = v.max(self.lo * 1e-3).max(1e-300);
            (v.log10() - self.lo.log10()) / (self.hi.log10() - self.lo.log10()).max(1e-12)
        } else {
            (v - self.lo) / (self.hi - self.lo).max(1e-12)
        };
        let t = t.clamp(-0.05, 1.05) as f32;
        if self.invert { 1.0 - t } else { t }
    }

    /// Where bars start (zero, or the axis minimum when zero is outside).
    pub fn base(&self) -> f64 {
        if self.log { self.lo } else { 0.0f64.clamp(self.lo, self.hi) }
    }
}

/// Number of ticks that suits an axis length.
pub fn tick_target(len: f32) -> usize {
    ((len / 55.0) as usize).clamp(2, 8)
}

/// How category labels are laid out.
#[derive(Clone, Debug)]
pub struct CatLayout {
    /// Show every `step`-th label.
    pub step: usize,
    /// Rotated labels (degrees, 0 = horizontal).
    pub angle: f32,
    /// Space the labels take below the plot.
    pub extent: f32,
    /// Max label length (ellipsize).
    pub max_len: f32,
    pub size: f32,
}

/// Lays out labels under bands of width `band`, with at most `max_extent` of height.
pub fn cat_layout(labels: &[String], band: f32, size: f32, max_extent: f32) -> CatLayout {
    let maxw = labels.iter().map(|l| text_width(l, size)).fold(0.0f32, f32::max);
    let band = band.max(1.0);
    if maxw <= band * 0.92 {
        return CatLayout { step: 1, angle: 0.0, extent: size * 1.5, max_len: band, size };
    }
    // Thin horizontal labels when that keeps at least a third of them.
    let thin = ((maxw + 8.0) / band).ceil() as usize;
    if thin <= 3 || labels.len() > 60 && maxw < 60.0 {
        return CatLayout { step: thin.max(1), angle: 0.0, extent: size * 1.5, max_len: band * thin as f32 - 6.0, size };
    }
    // Rotate 40°.
    let (s, c) = (40f32.to_radians().sin(), 40f32.to_radians().cos());
    let step = ((size * 1.7) / band).ceil().max(1.0) as usize;
    let max_len = ((max_extent - size * 0.8) / s).clamp(20.0, 160.0);
    let len = maxw.min(max_len);
    let _ = c;
    CatLayout { step, angle: -40.0, extent: (len * s + size * 1.2).min(max_extent.max(size * 1.5)), max_len, size }
}

/// Draws one category label at `x` (centre of its band) below `y`.
pub fn draw_cat_label(s: &mut Scene, lay: &CatLayout, x: f32, y: f32, label: &str, color: Color) {
    let text = ell(label, lay.size, lay.max_len);
    if lay.angle == 0.0 {
        s.text(x, y + 4.0, text, lay.size, color, Align::Center, Align::Start);
    } else {
        s.push(Shape::Text { x: x + lay.size * 0.3, y: y + 6.0, text, size: lay.size, color, bold: false, italic: false, h: Align::End, v: Align::Center, angle: lay.angle });
    }
}

/// An axis title rotated for a vertical axis.
pub fn vertical_title(s: &mut Scene, x: f32, cy: f32, text: &str, size: f32, color: Color, max_len: f32) {
    s.push(Shape::Text { x, y: cy, text: ell(text, size, max_len), size, color, bold: false, italic: false, h: Align::Center, v: Align::Center, angle: -90.0 });
}

/// One legend entry.
#[derive(Clone, Debug)]
pub struct LegendItem {
    pub label: String,
    pub color: Color,
    /// "Circle", "Square", "Line" (from `legend.marker`), or a line style for line series.
    pub line: bool,
    pub hit: Option<Hit>,
    pub dimmed: bool,
}

/// Draws the legend in `area` per `legend.position` and returns the remaining area.
pub fn legend(s: &mut Scene, o: &RenderOpts, items: &[LegendItem], title: &str, area: Rect) -> Rect {
    if items.is_empty() || !o.props.bool("legend.show", true) || area.w < 60.0 || area.h < 40.0 {
        return area;
    }
    let size = o.props.f32("legend.size", 10.0);
    let color = o.props.color_opt("legend.color").unwrap_or(o.theme.foreground_muted);
    let marker = o.props.text("legend.marker", "Circle");
    let title = if o.props.bool("legend.title", true) {
        let t = o.props.text("legend.titleText", "");
        if t.is_empty() { title.to_string() } else { t }
    } else {
        String::new()
    };
    let pos = o.props.text("legend.position", "Top");
    let row_h = size * 1.6;
    let mk = size * 0.75;
    let draw_marker = |s: &mut Scene, x: f32, cy: f32, it: &LegendItem| {
        let shape = if it.line || marker == "Line" {
            Shape::Rect { rect: Rect::new(x, cy - 1.5, mk + 4.0, 3.0), fill: Some(it.color), stroke: None, radius: 1.5 }
        } else if marker == "Square" {
            Shape::Rect { rect: Rect::new(x, cy - mk / 2.0, mk, mk), fill: Some(it.color), stroke: None, radius: 2.0 }
        } else {
            Shape::Circle { cx: x + mk / 2.0, cy, r: mk / 2.0, fill: Some(it.color), stroke: None }
        };
        match &it.hit {
            Some(h) => s.push_hit(shape, h.clone(), it.dimmed),
            None => s.push(shape),
        }
    };
    let item_w = |it: &LegendItem| mk + (if it.line { 4.0 } else { 0.0 }) + 5.0 + text_width(&it.label, size).min(150.0) + 14.0;
    if matches!(pos.as_str(), "Left" | "Right" | "Center right") {
        let w = items.iter().map(item_w).fold(text_width(&title, size), f32::max).min(area.w * 0.32).max(50.0);
        let x0 = if pos == "Left" { area.x } else { area.right() - w };
        let total_h = items.len() as f32 * row_h + if title.is_empty() { 0.0 } else { row_h };
        let mut y = if pos == "Center right" || total_h < area.h { area.y + ((area.h - total_h) / 2.0).max(0.0) } else { area.y };
        if !title.is_empty() {
            s.text_bold(x0, y + row_h / 2.0, ell(&title, size, w), size, o.theme.foreground, Align::Start, Align::Center);
            y += row_h;
        }
        for (i, it) in items.iter().enumerate() {
            if y + row_h > area.bottom() + 1.0 {
                let more = items.len() - i;
                s.text(x0, y + row_h / 2.0 - row_h, format!("+{more} more"), size * 0.9, color, Align::Start, Align::Center);
                break;
            }
            draw_marker(s, x0, y + row_h / 2.0, it);
            let tx = x0 + mk + if it.line { 4.0 } else { 0.0 } + 5.0;
            s.text(tx, y + row_h / 2.0, ell(&it.label, size, w - (tx - x0)), size, color, Align::Start, Align::Center);
            y += row_h;
        }
        return if pos == "Left" { Rect::new(area.x + w + 8.0, area.y, (area.w - w - 8.0).max(1.0), area.h) } else { Rect::new(area.x, area.y, (area.w - w - 8.0).max(1.0), area.h) };
    }
    // Horizontal flow, wrapping to at most 2 lines.
    let bottom = pos.starts_with("Bottom");
    let centered = pos.contains("center");
    let title_w = if title.is_empty() { 0.0 } else { text_width(&title, size) * 1.1 + 12.0 };
    let mut lines: Vec<Vec<usize>> = vec![vec![]];
    let mut x = title_w;
    for (i, it) in items.iter().enumerate() {
        let w = item_w(it);
        if x + w > area.w && lines.last().is_some_and(|l| !l.is_empty()) {
            if lines.len() >= 2 {
                break;
            }
            lines.push(vec![]);
            x = title_w;
        }
        if let Some(l) = lines.last_mut() {
            l.push(i);
        }
        x += w;
    }
    let shown: usize = lines.iter().map(Vec::len).sum();
    let h = lines.len() as f32 * row_h + 4.0;
    let y0 = if bottom { area.bottom() - h + 2.0 } else { area.y };
    if !title.is_empty() {
        s.text_bold(area.x, y0 + row_h / 2.0, &title, size, o.theme.foreground, Align::Start, Align::Center);
    }
    for (li, line) in lines.iter().enumerate() {
        let lw: f32 = line.iter().filter_map(|i| items.get(*i)).map(item_w).sum();
        let mut x = area.x + title_w + if centered { ((area.w - title_w - lw) / 2.0).max(0.0) } else { 0.0 };
        let cy = y0 + li as f32 * row_h + row_h / 2.0;
        for (k, i) in line.iter().enumerate() {
            let Some(it) = items.get(*i) else { continue };
            draw_marker(s, x, cy, it);
            let tx = x + mk + if it.line { 4.0 } else { 0.0 } + 5.0;
            let label = if li == lines.len() - 1 && k == line.len() - 1 && shown < items.len() { format!("{} (+{})", it.label, items.len() - shown) } else { it.label.clone() };
            s.text(tx, cy, ell(&label, size, 150.0), size, color, Align::Start, Align::Center);
            x += item_w(it);
        }
    }
    if bottom { Rect::new(area.x, area.y, area.w, (area.h - h - 4.0).max(1.0)) } else { Rect::new(area.x, area.y + h + 4.0, area.w, (area.h - h - 4.0).max(1.0)) }
}

/// Draws horizontal gridlines and value labels for a vertical value axis.
#[allow(clippy::too_many_arguments)]
pub fn value_axis_v(s: &mut Scene, o: &RenderOpts, sc: &Scale, plot: Rect, fmt: &ValueFmt, prefix: &str, right: bool, grid: bool) {
    let size = o.props.f32(&format!("{prefix}.size"), 10.0) * 0.92;
    let col = axis_text(o, prefix);
    let show = o.props.bool(&format!("{prefix}.show"), true);
    let gl = grid_line(o);
    for t in &sc.ticks {
        let y = plot.bottom() - plot.h * sc.frac(*t);
        if y < plot.y - 1.0 || y > plot.bottom() + 1.0 {
            continue;
        }
        if grid && o.props.bool("grid.horizontal", true) {
            s.line((plot.x, y), (plot.right(), y), gl.clone());
        }
        if show {
            if right {
                s.text(plot.right() + 6.0, y, fmt.short(*t), size, col, Align::Start, Align::Center);
            } else {
                s.text(plot.x - 6.0, y, fmt.short(*t), size, col, Align::End, Align::Center);
            }
        }
    }
}

/// Width the tick labels of a vertical value axis need.
pub fn value_axis_width(o: &RenderOpts, sc: &Scale, fmt: &ValueFmt, prefix: &str) -> f32 {
    if !o.props.bool(&format!("{prefix}.show"), true) {
        return 4.0;
    }
    let size = o.props.f32(&format!("{prefix}.size"), 10.0) * 0.92;
    sc.ticks.iter().map(|t| text_width(&fmt.short(*t), size)).fold(0.0, f32::max) + 10.0
}

/// Draws vertical gridlines and value labels for a horizontal value axis (bar charts).
pub fn value_axis_h(s: &mut Scene, o: &RenderOpts, sc: &Scale, plot: Rect, fmt: &ValueFmt, prefix: &str) {
    let size = o.props.f32(&format!("{prefix}.size"), 10.0) * 0.92;
    let col = axis_text(o, prefix);
    let show = o.props.bool(&format!("{prefix}.show"), true);
    let gl = grid_line(o);
    let mut last_right = f32::MIN;
    for t in &sc.ticks {
        let x = plot.x + plot.w * sc.frac(*t);
        if x < plot.x - 1.0 || x > plot.right() + 1.0 {
            continue;
        }
        if o.props.bool("grid.horizontal", true) {
            s.line((x, plot.y), (x, plot.bottom()), gl.clone());
        }
        let label = fmt.short(*t);
        let w = text_width(&label, size);
        if show && x - w / 2.0 > last_right + 4.0 {
            s.text(x, plot.bottom() + 5.0, label, size, col, Align::Center, Align::Start);
            last_right = x + w / 2.0;
        }
    }
}

/// A thin baseline (zero line) across the plot.
pub fn baseline(s: &mut Scene, o: &RenderOpts, a: (f32, f32), b: (f32, f32)) {
    let c = if o.theme.visual_background.luminance() < 0.4 { o.theme.foreground_muted.with_alpha(120) } else { Color::rgb(0xC4, 0xC9, 0xD2) };
    s.line(a, b, Line::solid(1.0, c));
}
