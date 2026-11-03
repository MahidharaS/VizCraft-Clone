//! Scatter and bubble charts: x/y values, size, legend colours, category labels, play axis
//! (last frame), clusters, anomalies, trend line (least squares of y on x), reference lines.

use super::RenderOpts;
use super::axis::*;
use super::common::*;
use crate::analytics::Computed;
use crate::color::Color;
use crate::doc::*;
use crate::scene::{Align, Line, Rect, Scene, Shape, text_width};
use crate::vquery::VisualData;

pub fn render(spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let (Some(xi), Some(yi)) = (d.first("x").filter(|i| d.cols.get(*i).is_some_and(|c| c.is_value)), d.first("y").filter(|i| d.cols.get(*i).is_some_and(|c| c.is_value))) else {
        return empty(o, "Add fields to X-axis and Y-axis");
    };
    let size_col = d.first("size").filter(|i| d.cols.get(*i).is_some_and(|c| c.is_value));
    let legend_col = d.first("legend").filter(|i| d.cols.get(*i).is_some_and(|c| !c.is_value));
    let details = d.first("details").filter(|i| d.cols.get(*i).is_some_and(|c| !c.is_value));
    // Play axis: show the last frame.
    let mut rows: Vec<usize> = (0..d.rows.len()).collect();
    let mut frame_label = None;
    if let Some(pc) = d.first("play") {
        let frames = d.distinct(pc);
        if let Some(last) = frames.last() {
            rows.retain(|r| d.cell(*r, pc).key() == last.key());
            frame_label = rows.first().map(|r| cat_text(d, *r, pc));
        }
    }
    rows.retain(|r| d.num(*r, xi).is_some_and(f64::is_finite) && d.num(*r, yi).is_some_and(f64::is_finite));
    if rows.is_empty() {
        return empty(o, "No data for the current filters");
    }
    let mut s = Scene::new(o.width, o.height);
    let mut area = Rect::new(2.0, 2.0, o.width - 4.0, o.height - 4.0);
    let clusters = o.analytics.iter().find_map(|a| match a {
        Computed::Clusters { assignment, k } => Some((assignment.clone(), *k)),
        _ => None,
    });
    // Colours: legend values / clusters / one colour.
    let legend_vals = legend_col.map(|c| d.distinct(c)).unwrap_or_default();
    let color_of = |r: usize| -> Color {
        if let Some(c) = o.props.point_colors.get(&color_key(d, r)) {
            return *c;
        }
        if let Some((a, _)) = &clusters {
            return o.theme.series_color(a.get(r).copied().unwrap_or(0));
        }
        if let Some(lc) = legend_col {
            let k = d.cell(r, lc).key();
            let i = legend_vals.iter().position(|v| v.key() == k).unwrap_or(0);
            return series_color(o, i, &crate::fx::point_key(&[(d.cols.get(lc).map(|c| c.field.clone()).unwrap_or(FieldRef::col("", "")), d.cell(r, lc).clone())]));
        }
        series_color(o, 0, "")
    };
    let mut items = Vec::new();
    if let Some((_, k)) = &clusters {
        for i in 0..*k {
            items.push(LegendItem { label: format!("Cluster {}", i + 1), color: o.theme.series_color(i), line: false, hit: None, dimmed: false });
        }
    } else if let Some(lc) = legend_col {
        for (i, v) in legend_vals.iter().enumerate() {
            let field = d.cols.get(lc).map(|c| c.field.clone()).unwrap_or(FieldRef::col("", ""));
            let point = vec![(field, v.clone())];
            let label = rows.iter().find(|r| d.cell(**r, lc).key() == v.key()).map(|r| cat_text(d, *r, lc)).unwrap_or_else(|| v.text());
            items.push(LegendItem { label, color: series_color(o, i, &crate::fx::point_key(&point)), line: false, hit: Some(crate::scene::Hit { point: point.clone(), tooltip: vec![] }), dimmed: dim(o, &point) });
        }
    }
    let ltitle = if clusters.is_some() { "Clusters".to_string() } else { legend_col.and_then(|c| d.cols.get(c)).map(|c| c.label.clone()).unwrap_or_default() };
    area = legend(&mut s, o, &items, &ltitle, area);
    if let Some(f) = &frame_label {
        s.text(area.right() - 4.0, area.y, f, 18.0, o.theme.foreground_muted.with_alpha(110), Align::End, Align::Start);
    }
    let xs: Vec<f64> = rows.iter().filter_map(|r| d.num(*r, xi)).collect();
    let ys: Vec<f64> = rows.iter().filter_map(|r| d.num(*r, yi)).collect();
    let (xlo, xhi) = (xs.iter().cloned().fold(f64::INFINITY, f64::min), xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
    let (mut ylo, mut yhi) = (ys.iter().cloned().fold(f64::INFINITY, f64::min), ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
    for a in o.analytics {
        if let Computed::ValueLine { value, .. } = a {
            ylo = ylo.min(*value);
            yhi = yhi.max(*value);
        }
    }
    let pad = |lo: f64, hi: f64| {
        let span = (hi - lo).abs().max(hi.abs() * 0.1).max(1e-9);
        (lo - span * 0.06, hi + span * 0.06)
    };
    let (xa, xb) = pad(xlo, xhi);
    let (ya, yb) = pad(ylo, yhi);
    let zero = |a: f64, b: f64| a >= 0.0 && a < (b - a) * 0.6;
    let xfmt = ValueFmt::of_col(d, xi).with_props(o, "xAxis");
    let yfmt = ValueFmt::of_col(d, yi).with_props(o, "yAxis");
    let ysc = Scale::from_props(o, "yAxis", ya, yb, zero(ya, yb), tick_target(area.h * 0.8));
    let ysize = o.props.f32("yAxis.size", 10.0) * 0.92;
    let yw = value_axis_width(o, &ysc, &yfmt, "yAxis");
    let titles = o.props.bool("xAxis.title", true) && area.h > 140.0;
    let ytitle = o.props.bool("yAxis.title", true) && area.w > 180.0;
    let left = area.x + yw + if ytitle { 16.0 } else { 0.0 };
    let bottom = ysize * 1.8 + if titles { 16.0 } else { 0.0 };
    let plot = Rect::new(left, area.y + 6.0, (area.right() - left - 10.0).max(10.0), (area.h - bottom - 6.0).max(10.0));
    let xsc = Scale::from_props(o, "xAxis", xa, xb, zero(xa, xb), tick_target(plot.w * 0.75));
    value_axis_v(&mut s, o, &ysc, plot, &yfmt, "yAxis", false, true);
    value_axis_h(&mut s, o, &xsc, plot, &xfmt, "xAxis");
    let px = |v: f64| plot.x + plot.w * xsc.frac(v);
    let py = |v: f64| plot.bottom() - plot.h * ysc.frac(v);
    if ysc.lo < 0.0 && ysc.hi > 0.0 {
        baseline(&mut s, o, (plot.x, py(0.0)), (plot.right(), py(0.0)));
    }
    if xsc.lo < 0.0 && xsc.hi > 0.0 {
        baseline(&mut s, o, (px(0.0), plot.y), (px(0.0), plot.bottom()));
    }
    // Reference lines (y).
    for a in o.analytics {
        if let Computed::ValueLine { value, label, color, width, dash } = a {
            let y = py(*value);
            s.line((plot.x, y), (plot.right(), y), Line::from_style(*width, *color, dash));
            s.text(plot.right() - 2.0, y - 2.0, format!("{label} {}", yfmt.short(*value)), 9.0, *color, Align::End, Align::End);
        }
    }
    // Trend: least squares y on x.
    if spec.analytics.iter().any(|l| l.kind.starts_with("Trend")) {
        let n = xs.len() as f64;
        let mx = xs.iter().sum::<f64>() / n;
        let my = ys.iter().sum::<f64>() / n;
        let sxx: f64 = xs.iter().map(|x| (x - mx).powi(2)).sum();
        let sxy: f64 = xs.iter().zip(ys.iter()).map(|(x, y)| (x - mx) * (y - my)).sum();
        if sxx > 0.0 {
            let b = sxy / sxx;
            let a = my - b * mx;
            let c = spec.analytics.iter().find(|l| l.kind.starts_with("Trend")).map(|l| l.color).unwrap_or(o.theme.foreground_muted);
            s.line((px(xlo), py(a + b * xlo)), (px(xhi), py(a + b * xhi)), Line::dashed(1.5, c));
        }
    }
    // Bubbles: bigger first.
    let smax = size_col.map(|c| rows.iter().filter_map(|r| d.num(*r, c)).fold(0.0f64, |a, b| a.max(b.abs()))).unwrap_or(0.0);
    let base_r = o.props.f32("markers.size", 6.0).max(1.5);
    let max_r = (plot.w.min(plot.h) / 11.0).clamp(base_r + 2.0, 40.0);
    let radius = |r: usize| -> f32 {
        match size_col {
            Some(c) if smax > 0.0 => {
                let v = d.num(r, c).unwrap_or(0.0).abs();
                (max_r * (v / smax).sqrt() as f32).max(3.0)
            }
            _ => base_r,
        }
    };
    let mut order = rows.clone();
    order.sort_by(|a, b| radius(*b).partial_cmp(&radius(*a)).unwrap_or(std::cmp::Ordering::Equal));
    let alpha = ((100.0 - o.props.f32("markers.transparency", 15.0).clamp(0.0, 100.0)) / 100.0 * 255.0) as u8;
    let shape = o.props.text("markers.shape", "Circle");
    let anomalies: Vec<usize> = o.analytics.iter().filter_map(|a| if let Computed::Anomalies { rows, .. } = a { Some(rows.clone()) } else { None }).flatten().collect();
    let mut placed: Vec<Rect> = Vec::new();
    let lsize = o.props.f32("labels.size", 9.0);
    let show_labels = o.props.bool("scatter.categoryLabels", false) || o.props.bool("labels.show", false);
    for r in &order {
        let (Some(x), Some(y)) = (d.num(*r, xi), d.num(*r, yi)) else { continue };
        let (cx, cy) = (px(x), py(y));
        let rad = radius(*r);
        let c = color_of(*r);
        let h = hit(d, *r, POINT_ROLES);
        let dimmed = dim(o, &h.point);
        s.push_hit(marker_shape(&shape, cx, cy, rad, c.with_alpha(alpha), Some(o.theme.visual_background.with_alpha(200))), h, dimmed);
        if anomalies.contains(r) {
            s.push(Shape::Circle { cx, cy, r: rad + 4.0, fill: None, stroke: Some(Line::solid(1.8, Color::rgb(0xDC, 0x26, 0x26))) });
        }
    }
    if show_labels && let Some(dc) = details {
        for r in &rows {
            let (Some(x), Some(y)) = (d.num(*r, xi), d.num(*r, yi)) else { continue };
            let (cx, cy) = (px(x), py(y));
            let t = ell(&cat_text(d, *r, dc), lsize, 110.0);
            let w = text_width(&t, lsize);
            let rad = radius(*r);
            let rect = Rect::new(cx + rad + 2.0, cy - lsize * 0.6, w, lsize * 1.2);
            if rect.right() < plot.right() + 20.0 && !placed.iter().any(|p| overlaps(p, &rect)) {
                s.text(rect.x, cy, t, lsize, o.theme.foreground_muted, Align::Start, Align::Center);
                placed.push(rect);
            }
        }
    }
    let muted = o.theme.foreground_muted;
    if titles {
        let t = d.cols.get(xi).map(|c| c.label.clone()).unwrap_or_default();
        s.text(plot.x + plot.w / 2.0, area.bottom() - 1.0, ell(&t, 10.0, plot.w), 10.0, muted, Align::Center, Align::End);
    }
    if ytitle {
        let t = d.cols.get(yi).map(|c| c.label.clone()).unwrap_or_default();
        vertical_title(&mut s, area.x + 6.0, plot.y + plot.h / 2.0, &t, 10.0, muted, plot.h);
    }
    s
}

pub fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
}
