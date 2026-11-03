//! Single-number visuals: card, multi-row card, KPI (value vs goal with trend) and gauge.

use super::RenderOpts;
use super::common::*;
use crate::color::Color;
use crate::doc::*;
use crate::scalar::Scalar;
use crate::scene::{Align, Hit, Line, Rect, Scene, Shape, text_width};
use crate::vquery::VisualData;
use std::f32::consts::PI;

/// The callout text of a value column (honours callout units / format).
fn callout_text(d: &VisualData, o: &RenderOpts, row: usize, col: usize, v: Option<f64>) -> String {
    match v.or_else(|| d.num(row, col)) {
        Some(n) if n.is_finite() => {
            let mut f = ValueFmt::of_col(d, col).with_props(o, "callout");
            if f.units == "Auto" && n.abs() < 1e5 && f.explicit.is_empty() {
                return f.full(n);
            }
            if f.units == "Auto" {
                f.decimals = -1;
            }
            f.short(n)
        }
        _ => {
            let c = d.cell(row, col);
            if c.is_null() { "(Blank)".into() } else { d.text(row, col) }
        }
    }
}

fn fit_size(text: &str, want: f32, max_w: f32, max_h: f32) -> f32 {
    let w = text_width(text, want).max(1.0);
    let s = if w > max_w { want * max_w / w } else { want };
    s.min(max_h).max(8.0)
}

pub fn card(_spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let mut s = Scene::new(o.width, o.height);
    let Some(vi) = d.values().first().copied().or_else(|| d.cols.first().map(|_| 0)) else { return empty(o, "Add a field to Fields") };
    let row = 0;
    let total = d.totals.get(vi).and_then(Scalar::as_f64);
    let v = if d.rows.len() == 1 { d.num(row, vi) } else { total.or_else(|| d.num(row, vi)) };
    let text = if d.rows.is_empty() && total.is_none() { "(Blank)".to_string() } else { callout_text(d, o, row, vi, v) };
    let align = o.props.text("card.align", "Center");
    let accent = o.props.bool("card.accentBar", false);
    let pad = if accent { 14.0 } else { 6.0 };
    let (x, h) = match align.as_str() {
        "Left" => (pad, Align::Start),
        "Right" => (o.width - 6.0, Align::End),
        _ => (o.width / 2.0, Align::Center),
    };
    if accent {
        s.push(Shape::Rect { rect: Rect::new(0.0, 4.0, 4.0, o.height - 8.0), fill: Some(o.props.color_opt("card.accentColor").unwrap_or(o.theme.accent)), stroke: None, radius: 2.0 });
    }
    let show_cat = o.props.bool("category.show", true) && o.height > 50.0;
    let csize = o.props.f32("category.size", 11.0);
    let reference = o.props.text("card.reference", "");
    let below = if show_cat { csize * 1.6 } else { 0.0 } + if reference.is_empty() { 0.0 } else { csize * 1.4 };
    let size = fit_size(&text, o.props.f32("callout.size", 36.0), o.width - pad * 2.0, (o.height - below) * 0.85);
    let cy = (o.height - below) / 2.0 + size * 0.05;
    let color = o.props.color_opt("callout.color").unwrap_or(o.theme.foreground);
    let point = d.identity(row, &[]);
    let tip = vec![(d.cols.get(vi).map(|c| c.label.clone()).unwrap_or_default(), text.clone())];
    s.push_hit(Shape::Text { x, y: cy, text, size, color, bold: o.props.bool("callout.bold", true), italic: false, h, v: Align::Center, angle: 0.0 }, Hit { point, tooltip: tip }, false);
    let mut y = cy + size * 0.55 + 2.0;
    if show_cat {
        let label = d.cols.get(vi).map(|c| c.label.clone()).unwrap_or_default();
        s.text(x, y, ell(&label, csize, o.width - pad * 2.0), csize, o.props.color_opt("category.color").unwrap_or(o.theme.foreground_muted), h, Align::Start);
        y += csize * 1.5;
    }
    if !reference.is_empty() {
        s.text(x, y, ell(&reference, csize * 0.9, o.width - pad * 2.0), csize * 0.9, o.theme.foreground_muted.with_alpha(190), h, Align::Start);
    }
    s
}

pub fn multi_row(_spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let mut s = Scene::new(o.width, o.height);
    let vals = d.values();
    let cats = d.categories();
    if vals.is_empty() && cats.is_empty() {
        return empty(o, "Add fields");
    }
    let vsize = (o.props.f32("callout.size", 36.0) * 0.5).clamp(10.0, 28.0);
    let lsize = o.props.f32("category.size", 11.0) * 0.9;
    let accent = o.props.color_opt("card.accentColor").unwrap_or(o.theme.accent);
    let rows = d.rows.len().max(1);
    let title_h = if cats.is_empty() { 0.0 } else { lsize * 1.8 };
    let row_h = title_h + vsize * 1.3 + lsize * 1.7 + 12.0;
    let cell_w = 130.0f32.max(vals.iter().map(|v| text_width(&d.cols.get(*v).map(|c| c.label.clone()).unwrap_or_default(), lsize) + 12.0).fold(0.0, f32::max)).min(o.width - 16.0);
    let per_line = ((o.width - 16.0) / cell_w).floor().max(1.0) as usize;
    let lines = vals.len().div_ceil(per_line).max(1);
    let row_h = row_h + (lines.saturating_sub(1)) as f32 * (vsize * 1.3 + lsize * 1.7 + 4.0);
    let mut y = 2.0;
    for r in 0..rows {
        if y > o.height {
            break;
        }
        let rect = Rect::new(2.0, y, o.width - 4.0, row_h - 6.0);
        if rows > 1 || !cats.is_empty() {
            s.push(Shape::Rect { rect, fill: Some(o.theme.visual_background), stroke: Some(Line::solid(1.0, o.theme.visual_border)), radius: 4.0 });
            s.push(Shape::Rect { rect: Rect::new(2.0, y, 3.0, row_h - 6.0), fill: Some(accent), stroke: None, radius: 1.5 });
        }
        let mut yy = y + 6.0;
        if !cats.is_empty() {
            let title = cats.iter().map(|c| cat_text(d, r, *c)).collect::<Vec<_>>().join(" · ");
            s.text_bold(14.0, yy + lsize * 0.7, ell(&title, lsize * 1.1, o.width - 28.0), lsize * 1.1, o.theme.foreground, Align::Start, Align::Center);
            yy += title_h;
        }
        for (i, v) in vals.iter().enumerate() {
            let (cx, cl) = ((i % per_line) as f32, (i / per_line) as f32);
            let x = 14.0 + cx * cell_w;
            let vy = yy + cl * (vsize * 1.3 + lsize * 1.7 + 4.0);
            let t = callout_text(d, o, r, *v, None);
            s.text_bold(x, vy, ell(&t, vsize, cell_w - 8.0), vsize, o.props.color_opt("callout.color").unwrap_or(o.theme.foreground), Align::Start, Align::Start);
            let label = d.cols.get(*v).map(|c| c.label.clone()).unwrap_or_default();
            s.text(x, vy + vsize * 1.25, ell(&label, lsize, cell_w - 8.0), lsize, o.theme.foreground_muted, Align::Start, Align::Start);
        }
        let point = d.identity(r, &[]);
        let dimmed = dim(o, &point);
        if !point.is_empty() {
            s.push_hit(Shape::Rect { rect, fill: None, stroke: None, radius: 0.0 }, Hit { point, tooltip: tooltip(d, r) }, dimmed);
        }
        y += row_h;
    }
    s
}

pub fn kpi(_spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let mut s = Scene::new(o.width, o.height);
    let Some(vi) = d.first("values").filter(|c| d.cols.get(*c).is_some_and(|x| x.is_value)).or_else(|| d.values().first().copied()) else { return empty(o, "Add a field to Value") };
    let ti = d.first("target").filter(|c| d.cols.get(*c).is_some_and(|x| x.is_value));
    let axis = d.first("axis");
    // Value and goal: the totals over the period (or the last row without an axis total).
    let value = d.totals.get(vi).and_then(Scalar::as_f64).or_else(|| d.rows.last().and_then(|r| r.get(vi)).and_then(Scalar::as_f64));
    let goal = ti.and_then(|t| d.totals.get(t).and_then(Scalar::as_f64).or_else(|| d.rows.last().and_then(|r| r.get(t)).and_then(Scalar::as_f64)));
    let Some(value) = value else { return empty(o, "(Blank)") };
    let high_good = o.props.text("kpi.direction", "Good") != "Bad";
    let good = o.props.color("kpi.good", o.theme.good);
    let bad = o.props.color("kpi.bad", o.theme.bad);
    let neutral = o.props.color("kpi.neutral", o.theme.neutral);
    let status = goal.map(|g| {
        let better = if high_good { value >= g } else { value <= g };
        if (value - g).abs() < 1e-9 { neutral } else if better { good } else { bad }
    });
    let color = status.unwrap_or(o.theme.foreground);
    // Trend area (bottom half).
    if o.props.bool("kpi.trend", true)
        && axis.is_some()
        && d.rows.len() >= 2
    {
        let ys: Vec<f64> = (0..d.rows.len()).map(|r| d.num(r, vi).unwrap_or(f64::NAN)).collect();
        let lo = ys.iter().cloned().filter(|v| v.is_finite()).fold(f64::INFINITY, f64::min);
        let hi = ys.iter().cloned().filter(|v| v.is_finite()).fold(f64::NEG_INFINITY, f64::max);
        if lo.is_finite() && hi.is_finite() {
            let top = o.height * 0.55;
            let h = o.height - top;
            let n = ys.len();
            let pts: Vec<(f32, f32)> = ys.iter().enumerate().filter(|(_, v)| v.is_finite()).map(|(i, v)| (o.width * i as f32 / (n - 1).max(1) as f32, top + h * (1.0 - ((v - lo) / (hi - lo).max(1e-12)) as f32 * 0.85))).collect();
            let mut poly = pts.clone();
            poly.push((o.width, o.height));
            poly.push((0.0, o.height));
            let tc = status.unwrap_or(o.theme.accent);
            s.push(Shape::Path { points: poly, closed: true, fill: Some(tc.with_alpha(40)), stroke: None });
            s.push(Shape::Path { points: pts, closed: false, fill: None, stroke: Some(Line::solid(1.5, tc.with_alpha(170))) });
        }
    }
    let text = callout_text(d, o, 0, vi, Some(value));
    let icon = match goal {
        Some(g) if value > g => "▲ ",
        Some(g) if value < g => "▼ ",
        _ => "",
    };
    let full = format!("{icon}{text}");
    let size = fit_size(&full, o.props.f32("callout.size", 36.0), o.width - 16.0, o.height * 0.38);
    s.push_hit(Shape::Text { x: o.width / 2.0, y: o.height * 0.26, text: full, size, color, bold: true, italic: false, h: Align::Center, v: Align::Center, angle: 0.0 }, Hit { point: vec![], tooltip: vec![(d.cols.get(vi).map(|c| c.label.clone()).unwrap_or_default(), text)] }, false);
    if let Some(g) = goal
        && o.props.bool("kpi.goal", true)
    {
        let gtext = callout_text(d, o, 0, ti.unwrap_or(vi), Some(g));
        let mut line = format!("Goal: {gtext}");
        if o.props.bool("kpi.distance", true) && g != 0.0 {
            line += &format!("  ({:+.1}%)", (value - g) / g.abs() * 100.0);
        }
        let ls = (size * 0.32).clamp(8.0, 13.0);
        s.text(o.width / 2.0, o.height * 0.26 + size * 0.62, ell(&line, ls, o.width - 10.0), ls, o.theme.foreground_muted, Align::Center, Align::Start);
    }
    s
}

pub fn gauge(_spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let mut s = Scene::new(o.width, o.height);
    let Some(vi) = d.first("values").filter(|c| d.cols.get(*c).is_some_and(|x| x.is_value)).or_else(|| d.values().first().copied()) else { return empty(o, "Add a field to Value") };
    let num = |role: &str, prop: &str| -> Option<f64> {
        o.props.text(prop, "").trim().parse::<f64>().ok().or_else(|| d.first(role).filter(|c| d.cols.get(*c).is_some_and(|x| x.is_value)).and_then(|c| d.num(0, c).or_else(|| d.totals.get(c).and_then(Scalar::as_f64))))
    };
    let Some(value) = d.num(0, vi).or_else(|| d.totals.get(vi).and_then(Scalar::as_f64)) else { return empty(o, "(Blank)") };
    let target = num("target", "gauge.target");
    let min = num("min", "gauge.min").unwrap_or(0.0f64.min(value));
    let max = num("max", "gauge.max").unwrap_or_else(|| {
        let m = value.max(target.unwrap_or(0.0)) * if target.is_some() { 1.25 } else { 2.0 };
        let st = nice_step(m / 4.0);
        (m / st).ceil() * st
    });
    let max = if max <= min { min + 1.0 } else { max };
    let fmt = ValueFmt::of_col(d, vi);
    let lsize = 9.5;
    let r = (o.width / 2.0 - 12.0).min(o.height - lsize * 2.6 - 12.0).max(10.0);
    let (cx, cy) = (o.width / 2.0, (o.height + r) / 2.0 - lsize * 0.6);
    let thick = (r * 0.28).clamp(6.0, 34.0);
    let ang = |v: f64| -> f32 { -PI / 2.0 + PI * ((v - min) / (max - min)).clamp(0.0, 1.0) as f32 };
    let track = o.props.color("gauge.track", if o.theme.visual_background.luminance() < 0.4 { o.theme.visual_border } else { Color::rgb(0xE5, 0xE7, 0xEB) });
    let fill = o.props.color_opt("gauge.fill").or(o.props.color_opt("dataColors.default")).unwrap_or(o.theme.series_color(0));
    s.push(Shape::Arc { cx, cy, r_inner: r - thick, r_outer: r, start: -PI / 2.0, end: PI / 2.0, fill: track, stroke: None });
    let a = ang(value);
    if a > -PI / 2.0 + 1e-3 {
        s.push(Shape::Arc { cx, cy, r_inner: r - thick, r_outer: r, start: -PI / 2.0, end: a, fill, stroke: None });
    }
    s.push_hit(Shape::Rect { rect: Rect::new(cx - r, cy - r, r * 2.0, r), fill: None, stroke: None, radius: 0.0 }, Hit { point: vec![], tooltip: tooltip(d, 0) }, false);
    if let Some(t) = target {
        let ta = ang(t);
        let tc = o.props.color("gauge.targetColor", o.theme.foreground);
        let (p0, p1) = ((cx + (r - thick - 4.0) * ta.sin(), cy - (r - thick - 4.0) * ta.cos()), (cx + (r + 4.0) * ta.sin(), cy - (r + 4.0) * ta.cos()));
        s.line(p0, p1, Line::solid(2.5, tc));
        let (lx, ly) = (cx + (r + 8.0) * ta.sin(), cy - (r + 8.0) * ta.cos());
        let h = if ta > 0.15 { Align::Start } else if ta < -0.15 { Align::End } else { Align::Center };
        s.text(lx, ly, fmt.short(t), lsize, o.theme.foreground_muted, h, Align::End);
    }
    let text = callout_text(d, o, 0, vi, Some(value));
    let size = fit_size(&text, (r * 0.42).min(o.props.f32("callout.size", 36.0)), (r - thick) * 1.7, r * 0.5);
    s.text_bold(cx, cy - size * 0.1, text, size, o.props.color_opt("callout.color").unwrap_or(o.theme.foreground), Align::Center, Align::End);
    s.text(cx - r + thick / 2.0, cy + 4.0, fmt.short(min), lsize, o.theme.foreground_muted, Align::Center, Align::Start);
    s.text(cx + r - thick / 2.0, cy + 4.0, fmt.short(max), lsize, o.theme.foreground_muted, Align::Center, Align::Start);
    s
}
