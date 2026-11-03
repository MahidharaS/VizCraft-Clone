//! Shared chart helpers and simple renderers.

use super::RenderOpts;
use crate::color::Color;
use crate::doc::*;
use crate::scalar::Scalar;
use crate::scene::{Align, Hit, Line, Rect, Scene, Shape, ellipsize, text_width};
use crate::vquery::VisualData;

/// A "nice" axis: ticks covering [lo, hi].
pub fn nice_ticks(lo: f64, hi: f64, target: usize) -> Vec<f64> {
    let (lo, hi) = if lo == hi { (lo - 1.0, hi + 1.0) } else { (lo.min(hi), lo.max(hi)) };
    let span = hi - lo;
    let raw = span / target.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    let step = mag * if norm < 1.5 { 1.0 } else if norm < 3.0 { 2.0 } else if norm < 7.0 { 5.0 } else { 10.0 };
    let start = (lo / step).floor() * step;
    let mut out = Vec::new();
    let mut v = start;
    while v <= hi + step * 0.5 && out.len() < 50 {
        out.push((v / step).round() * step);
        v += step;
    }
    out
}

pub fn short_num(v: f64, units: &str) -> String {
    match units {
        "None" => crate::numfmt::format(v, "#,0.##"),
        "Thousands" => format!("{}K", crate::numfmt::format(v / 1e3, "#,0.#")),
        "Millions" => format!("{}M", crate::numfmt::format(v / 1e6, "#,0.#")),
        "Billions" => format!("{}bn", crate::numfmt::format(v / 1e9, "#,0.#")),
        "Trillions" => format!("{}T", crate::numfmt::format(v / 1e12, "#,0.#")),
        _ => crate::numfmt::compact(v, if v.abs() >= 1000.0 { 1 } else { 2 }).replace(".0K", "K").replace(".0M", "M"),
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

fn is_selected(o: &RenderOpts, point: &[(FieldRef, Scalar)]) -> bool {
    o.selected.iter().any(|p| p.as_slice() == point)
}

/// Dimmed when something else is selected in this visual.
pub fn dim(o: &RenderOpts, point: &[(FieldRef, Scalar)]) -> bool {
    !o.selected.is_empty() && !is_selected(o, point)
}

pub fn tooltip(d: &VisualData, row: usize) -> Vec<(String, String)> {
    d.cols.iter().enumerate().map(|(i, c)| (c.label.clone(), d.text(row, i))).collect()
}

/// Minimal renderers used until a kind has its dedicated renderer.
pub fn render_basic(spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    match spec.kind {
        VisualKind::Card | VisualKind::Kpi | VisualKind::Gauge => card(d, o),
        VisualKind::Table | VisualKind::Matrix | VisualKind::MultiRowCard => table(d, o),
        VisualKind::Slicer => slicer(spec, d, o),
        VisualKind::Pie | VisualKind::Donut => pie(spec, d, o),
        _ => columns(spec, d, o),
    }
}

fn card(d: &VisualData, o: &RenderOpts) -> Scene {
    let mut s = Scene::new(o.width, o.height);
    let Some(vi) = d.values().first().copied() else { return super::message(o, "Add a value") };
    let text = d.text(0, vi);
    let size = o.props.f32("callout.size", 36.0).min(o.height * 0.6);
    s.text_bold(o.width / 2.0, o.height / 2.0 - 6.0, text, size, o.props.color("callout.color", o.theme.foreground), Align::Center, Align::Center);
    if o.props.bool("category.show", true) {
        let label = d.cols.get(vi).map(|c| c.label.clone()).unwrap_or_default();
        s.text(o.width / 2.0, o.height / 2.0 + size * 0.55, label, o.props.f32("category.size", 11.0), o.theme.foreground_muted, Align::Center, Align::Start);
    }
    s
}

fn table(d: &VisualData, o: &RenderOpts) -> Scene {
    let mut s = Scene::new(o.width, o.height);
    let size = o.props.f32("values.size", 10.0);
    let row_h = size * 2.0;
    let n = d.cols.len().max(1);
    let cw = o.width / n as f32;
    let hb = o.props.color("headers.background", o.theme.visual_background);
    s.rect(Rect::new(0.0, 0.0, o.width, row_h), hb);
    for (i, c) in d.cols.iter().enumerate() {
        s.text_bold(i as f32 * cw + 4.0, row_h / 2.0, ellipsize(&c.label, size, cw - 8.0), size, o.theme.foreground, Align::Start, Align::Center);
    }
    s.line((0.0, row_h), (o.width, row_h), Line::solid(1.0, o.theme.foreground_muted));
    for r in 0..d.rows.len() {
        let y = row_h * (r as f32 + 1.0);
        if y + row_h > o.height {
            break;
        }
        if r % 2 == 1 && o.props.bool("values.banded", true) {
            s.rect(Rect::new(0.0, y, o.width, row_h), o.theme.background.with_alpha(160));
        }
        for (i, c) in d.cols.iter().enumerate() {
            let t = d.text(r, i);
            let (x, h) = if c.is_value { (i as f32 * cw + cw - 4.0, Align::End) } else { (i as f32 * cw + 4.0, Align::Start) };
            s.text(x, y + row_h / 2.0, ellipsize(&t, size, cw - 8.0), size, o.theme.foreground, h, Align::Center);
        }
        let point = d.identity(r, &[]);
        let dimmed = dim(o, &point);
        s.push_hit(Shape::Rect { rect: Rect::new(0.0, y, o.width, row_h), fill: None, stroke: None, radius: 0.0 }, Hit { point, tooltip: tooltip(d, r) }, dimmed);
    }
    s
}

fn slicer(spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let mut s = Scene::new(o.width, o.height);
    let size = o.props.f32("values.size", 11.0);
    let row_h = size * 2.0;
    for r in 0..d.rows.len() {
        let y = r as f32 * row_h;
        if y + row_h > o.height {
            break;
        }
        let v = d.cell(r, 0).clone();
        let on = spec.selection.iter().any(|x| x.key() == v.key());
        s.push(Shape::Rect { rect: Rect::new(2.0, y + row_h / 2.0 - 6.0, 12.0, 12.0), fill: if on { Some(o.theme.accent) } else { None }, stroke: Some(Line::solid(1.0, o.theme.foreground_muted)), radius: 2.0 });
        s.text(20.0, y + row_h / 2.0, d.text(r, 0), size, o.theme.foreground, Align::Start, Align::Center);
        s.actions.push((Rect::new(0.0, y, o.width, row_h), format!("slicer:{}", serde_json::to_string(&v).unwrap_or_default())));
    }
    s
}

fn pie(spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let mut s = Scene::new(o.width, o.height);
    let Some(vi) = d.values().first().copied() else { return super::message(o, "Add values") };
    let total: f64 = d.rows.iter().filter_map(|r| r.get(vi)?.as_f64()).filter(|v| *v > 0.0).sum();
    if total <= 0.0 {
        return super::message(o, "No data");
    }
    let r = (o.width.min(o.height) / 2.0 - 16.0).max(10.0);
    let (cx, cy) = (o.width / 2.0, o.height / 2.0);
    let inner = if spec.kind == VisualKind::Donut { r * o.props.f32("donut.inner", 55.0) / 100.0 } else { 0.0 };
    let mut a = 0.0f32;
    for row in 0..d.rows.len() {
        let v = d.num(row, vi).unwrap_or(0.0).max(0.0);
        let sweep = (v / total) as f32 * std::f32::consts::TAU;
        let point = d.identity(row, &[]);
        let key = crate::fx::point_key(&point);
        let color = series_color(o, row, &key);
        let dimmed = dim(o, &point);
        s.push_hit(Shape::Arc { cx, cy, r_inner: inner, r_outer: r, start: a, end: a + sweep, fill: color, stroke: Some(Line::solid(1.0, o.theme.visual_background)) }, Hit { point, tooltip: tooltip(d, row) }, dimmed);
        a += sweep;
    }
    s
}

fn columns(spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let mut s = Scene::new(o.width, o.height);
    let vals = d.values();
    let Some(ai) = d.categories().first().copied() else { return card(d, o) };
    if vals.is_empty() {
        return super::message(o, "Add a value");
    }
    let horizontal = matches!(spec.kind, VisualKind::StackedBar | VisualKind::ClusteredBar | VisualKind::StackedBar100);
    let line = matches!(spec.kind, VisualKind::Line | VisualKind::Area | VisualKind::StackedArea);
    let cats = d.distinct(ai);
    let n = cats.len().max(1);
    let max = d.rows.iter().flat_map(|r| vals.iter().filter_map(|v| r.get(*v)?.as_f64())).fold(0.0f64, f64::max);
    let min = d.rows.iter().flat_map(|r| vals.iter().filter_map(|v| r.get(*v)?.as_f64())).fold(0.0f64, f64::min);
    let ticks = nice_ticks(min, max, 5);
    let lo = ticks.first().copied().unwrap_or(0.0);
    let hi = ticks.last().copied().unwrap_or(1.0).max(lo + 1e-9);
    let label_w = if horizontal { cats.iter().map(|c| text_width(&c.text(), 10.0)).fold(0.0, f32::max).min(o.width * 0.35) + 8.0 } else { 36.0 };
    let plot = Rect::new(label_w, 4.0, (o.width - label_w - 8.0).max(1.0), (o.height - 24.0).max(1.0));
    let grid = o.props.color("grid.color", Color::rgb(0xE6, 0xE8, 0xEC));
    let scale = |v: f64| -> f32 { ((v - lo) / (hi - lo)) as f32 };
    for t in &ticks {
        if horizontal {
            let x = plot.x + plot.w * scale(*t);
            s.line((x, plot.y), (x, plot.bottom()), Line::solid(1.0, grid));
            s.text(x, plot.bottom() + 4.0, short_num(*t, "Auto"), 9.0, o.theme.foreground_muted, Align::Center, Align::Start);
        } else {
            let y = plot.bottom() - plot.h * scale(*t);
            s.line((plot.x, y), (plot.right(), y), Line::solid(1.0, grid));
            s.text(plot.x - 4.0, y, short_num(*t, "Auto"), 9.0, o.theme.foreground_muted, Align::End, Align::Center);
        }
    }
    let band = if horizontal { plot.h } else { plot.w } / n as f32;
    let mut pts: Vec<Vec<(f32, f32)>> = vec![Vec::new(); vals.len()];
    for (ci, c) in cats.iter().enumerate() {
        let rows: Vec<usize> = (0..d.rows.len()).filter(|r| d.cell(*r, ai).key() == c.key()).collect();
        let start = ci as f32 * band;
        let label = ellipsize(&c.text(), 10.0, if horizontal { label_w - 6.0 } else { band.max(20.0) });
        if horizontal {
            s.text(plot.x - 4.0, plot.y + start + band / 2.0, label, 10.0, o.theme.foreground_muted, Align::End, Align::Center);
        } else if n <= 40 || ci % (n / 20).max(1) == 0 {
            s.text(plot.x + start + band / 2.0, plot.bottom() + 4.0, label, 10.0, o.theme.foreground_muted, Align::Center, Align::Start);
        }
        let Some(&r) = rows.first() else { continue };
        let k = vals.len() as f32;
        for (vi, v) in vals.iter().enumerate() {
            let val = d.num(r, *v).unwrap_or(0.0);
            let point = d.identity(r, &["axis"]);
            let key = crate::fx::point_key(&point);
            let color = if vals.len() == 1 { series_color(o, 0, &key) } else { series_color(o, vi, &format!("series:{vi}")) };
            let dimmed = dim(o, &point);
            let hit = Hit { point, tooltip: tooltip(d, r) };
            if line {
                let x = plot.x + start + band / 2.0;
                let y = plot.bottom() - plot.h * scale(val);
                if let Some(p) = pts.get_mut(vi) {
                    p.push((x, y));
                }
                s.push_hit(Shape::Circle { cx: x, cy: y, r: 3.0, fill: Some(color), stroke: None }, hit, dimmed);
            } else {
                let inner = band * 0.75;
                let bw = inner / k;
                let off = start + (band - inner) / 2.0 + bw * vi as f32;
                let z = scale(0.0f64.max(lo));
                let rect = if horizontal {
                    let x0 = plot.x + plot.w * z;
                    let x1 = plot.x + plot.w * scale(val);
                    Rect::new(x0.min(x1), plot.y + off, (x1 - x0).abs(), bw)
                } else {
                    let y0 = plot.bottom() - plot.h * z;
                    let y1 = plot.bottom() - plot.h * scale(val);
                    Rect::new(plot.x + off, y0.min(y1), bw, (y1 - y0).abs())
                };
                s.push_hit(Shape::Rect { rect, fill: Some(color), stroke: None, radius: o.props.f32("plot.cornerRadius", 2.0) }, hit, dimmed);
            }
        }
    }
    for (vi, p) in pts.into_iter().enumerate() {
        if p.len() > 1 {
            s.push(Shape::Path { points: p, closed: false, fill: None, stroke: Some(Line::solid(o.props.f32("lines.width", 2.5), series_color(o, vi, &format!("series:{vi}")))) });
        }
    }
    s
}
