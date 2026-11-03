//! Waterfall (increase / decrease / total colours, breakdown) and funnel charts.

use std::collections::HashMap;

use super::RenderOpts;
use super::axis::*;
use super::common::*;
use crate::color::Color;
use crate::doc::*;
use crate::scene::{Align, Hit, Line, Rect, Scene, Shape, text_width};
use crate::vquery::VisualData;

#[derive(Clone, Copy, PartialEq)]
enum StepKind {
    Increase,
    Decrease,
    Total,
}

struct Step {
    label: String,
    start: f64,
    end: f64,
    kind: StepKind,
    hit: Option<Hit>,
}

pub fn waterfall(_spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let Some(vi) = d.values().first().copied() else { return empty(o, "Add a field to Y-axis") };
    let axis = d.role("axis");
    if d.rows.is_empty() {
        return empty(o, "No data for the current filters");
    }
    let fmt = ValueFmt::of_col(d, vi).with_props(o, "yAxis");
    let breakdown = d.first("breakdown");
    let cat_key = |r: usize| axis.iter().map(|c| d.cell(r, *c).key()).collect::<Vec<_>>().join("\u{1}");
    let cat_label = |r: usize| axis.iter().map(|c| cat_text(d, r, *c)).collect::<Vec<_>>().join(" ");
    let mut steps: Vec<Step> = Vec::new();
    match breakdown {
        None => {
            let mut run = 0.0;
            for r in 0..d.rows.len() {
                let v = d.num(r, vi).filter(|v| v.is_finite()).unwrap_or(0.0);
                steps.push(Step { label: cat_label(r), start: run, end: run + v, kind: if v >= 0.0 { StepKind::Increase } else { StepKind::Decrease }, hit: Some(hit(d, r, POINT_ROLES)) });
                run += v;
            }
            if o.props.bool("waterfall.showTotal", true) {
                steps.push(Step { label: "Total".into(), start: 0.0, end: run, kind: StepKind::Total, hit: None });
            }
        }
        Some(bc) => {
            // Totals per category and per (category, breakdown).
            let mut cats: Vec<(String, String, usize)> = Vec::new();
            let mut per: HashMap<(String, String), f64> = HashMap::new();
            let mut names: HashMap<String, String> = HashMap::new();
            let mut tot: HashMap<String, f64> = HashMap::new();
            for r in 0..d.rows.len() {
                let k = cat_key(r);
                if !cats.iter().any(|c| c.0 == k) {
                    cats.push((k.clone(), cat_label(r), r));
                }
                let b = d.cell(r, bc).key();
                names.entry(b.clone()).or_insert_with(|| cat_text(d, r, bc));
                let v = d.num(r, vi).filter(|v| v.is_finite()).unwrap_or(0.0);
                *per.entry((k.clone(), b)).or_default() += v;
                *tot.entry(k).or_default() += v;
            }
            let mut bkeys: Vec<String> = names.keys().cloned().collect();
            bkeys.sort();
            for (i, (k, label, r)) in cats.iter().enumerate() {
                let t = tot.get(k).copied().unwrap_or(0.0);
                if i > 0 {
                    let (pk, _, _) = cats.get(i - 1).cloned().unwrap_or_default();
                    let mut run = tot.get(&pk).copied().unwrap_or(0.0);
                    let mut deltas: Vec<(String, f64)> = bkeys.iter().map(|b| (b.clone(), per.get(&(k.clone(), b.clone())).copied().unwrap_or(0.0) - per.get(&(pk.clone(), b.clone())).copied().unwrap_or(0.0))).collect();
                    deltas.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap_or(std::cmp::Ordering::Equal));
                    let keep = 5;
                    let other: f64 = deltas.iter().skip(keep).map(|x| x.1).sum();
                    for (b, dv) in deltas.iter().take(keep) {
                        let name = names.get(b).cloned().unwrap_or_default();
                        steps.push(Step { label: name.clone(), start: run, end: run + dv, kind: if *dv >= 0.0 { StepKind::Increase } else { StepKind::Decrease }, hit: Some(Hit { point: d.identity(*r, &["breakdown"]).into_iter().map(|(f, _)| (f, crate::scalar::Scalar::Text(name.clone()))).collect(), tooltip: vec![(name.clone(), fmt.full(*dv))] }) });
                        run += dv;
                    }
                    if deltas.len() > keep {
                        steps.push(Step { label: "Other".into(), start: run, end: run + other, kind: if other >= 0.0 { StepKind::Increase } else { StepKind::Decrease }, hit: None });
                    }
                }
                steps.push(Step { label: label.clone(), start: 0.0, end: t, kind: StepKind::Total, hit: Some(Hit { point: d.identity(*r, &["axis"]), tooltip: vec![(label.clone(), fmt.full(t))] }) });
            }
        }
    }
    let mut s = Scene::new(o.width, o.height);
    let mut area = Rect::new(2.0, 2.0, o.width - 4.0, o.height - 4.0);
    let inc = o.props.color("waterfall.increase", Color::rgb(0x16, 0xA3, 0x4A));
    let dec = o.props.color("waterfall.decrease", Color::rgb(0xDC, 0x26, 0x26));
    let totc = o.props.color("waterfall.total", o.theme.series_color(0));
    let items = vec![
        super::axis::LegendItem { label: "Increase".into(), color: inc, line: false, hit: None, dimmed: false },
        super::axis::LegendItem { label: "Decrease".into(), color: dec, line: false, hit: None, dimmed: false },
        super::axis::LegendItem { label: "Total".into(), color: totc, line: false, hit: None, dimmed: false },
    ];
    area = legend(&mut s, o, &items, "", area);
    let lo = steps.iter().map(|x| x.start.min(x.end)).fold(0.0f64, f64::min);
    let hi = steps.iter().map(|x| x.start.max(x.end)).fold(0.0f64, f64::max);
    let sc = Scale::from_props(o, "yAxis", lo, hi, true, tick_target(area.h * 0.8));
    let yw = value_axis_width(o, &sc, &fmt, "yAxis");
    let labels: Vec<String> = steps.iter().map(|x| x.label.clone()).collect();
    let pw = (area.w - yw - 8.0).max(10.0);
    let n = steps.len().max(1);
    let band = pw / n as f32;
    let xsize = o.props.f32("xAxis.size", 10.0) * 0.95;
    let lay = cat_layout(&labels, band, xsize, area.h * 0.28);
    let labels_on = o.props.bool("labels.show", true);
    let lsize = o.props.f32("labels.size", 9.0);
    let top = if labels_on { lsize * 1.6 } else { 6.0 };
    let plot = Rect::new(area.x + yw, area.y + top, pw, (area.h - lay.extent - top).max(10.0));
    value_axis_v(&mut s, o, &sc, plot, &fmt, "yAxis", false, true);
    let y = |v: f64| plot.bottom() - plot.h * sc.frac(v);
    let bw = band * (1.0 - o.props.f32("plot.innerPadding", 20.0).clamp(0.0, 90.0) / 100.0);
    let radius = o.props.f32("plot.cornerRadius", 2.0);
    let lfmt = fmt.clone().with_props(o, "labels");
    let conn = Line::from_style(1.0, o.theme.foreground_muted.with_alpha(120), "dot");
    for (i, st) in steps.iter().enumerate() {
        let x = plot.x + band * i as f32 + (band - bw) / 2.0;
        let (a, b) = (y(st.start), y(st.end));
        let rect = Rect::new(x, a.min(b), bw, (a - b).abs().max(1.0));
        let color = data_alpha(o, match st.kind {
            StepKind::Increase => inc,
            StepKind::Decrease => dec,
            StepKind::Total => totc,
        });
        let shape = Shape::Rect { rect, fill: Some(color), stroke: None, radius };
        match &st.hit {
            Some(h) => {
                let dimmed = dim(o, &h.point);
                s.push_hit(shape, h.clone(), dimmed);
            }
            None => s.push(shape),
        }
        if i + 1 < steps.len() {
            let nx = plot.x + band * (i + 1) as f32 + (band - bw) / 2.0;
            s.line((x + bw, b), (nx, b), conn.clone());
        }
        if labels_on {
            let dv = if st.kind == StepKind::Total { st.end } else { st.end - st.start };
            let t = lfmt.short(dv);
            if text_width(&t, lsize) < band * 1.15 {
                let up = st.end >= st.start;
                if up {
                    s.text(x + bw / 2.0, rect.y - 2.0, t, lsize, o.theme.foreground, Align::Center, Align::End);
                } else {
                    s.text(x + bw / 2.0, rect.bottom() + 2.0, t, lsize, o.theme.foreground, Align::Center, Align::Start);
                }
            }
        }
    }
    baseline(&mut s, o, (plot.x, y(0.0)), (plot.right(), y(0.0)));
    if o.props.bool("xAxis.show", true) {
        let col = axis_text(o, "xAxis");
        for (i, l) in labels.iter().enumerate() {
            if i % lay.step.max(1) == 0 {
                draw_cat_label(&mut s, &lay, plot.x + band * (i as f32 + 0.5), plot.bottom(), l, col);
            }
        }
    }
    s
}

pub fn funnel(_spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let Some(vi) = d.values().first().copied() else { return empty(o, "Add a field to Values") };
    let ax = d.first("axis");
    let rows: Vec<usize> = (0..d.rows.len()).filter(|r| d.num(*r, vi).is_some_and(|v| v.is_finite())).collect();
    if rows.is_empty() {
        return empty(o, "No data for the current filters");
    }
    let mut s = Scene::new(o.width, o.height);
    let fmt = ValueFmt::of_col(d, vi).with_props(o, "labels");
    let max = rows.iter().filter_map(|r| d.num(*r, vi)).fold(0.0f64, |a, b| a.max(b.abs())).max(1e-12);
    let first = rows.first().and_then(|r| d.num(*r, vi)).unwrap_or(max);
    let size = o.props.f32("labels.size", 9.0).max(9.0);
    let cat_w = ax.map(|c| rows.iter().map(|r| text_width(&cat_text(d, *r, c), size + 1.0)).fold(0.0, f32::max).min(o.width * 0.3) + 12.0).unwrap_or(0.0);
    let plot = Rect::new(cat_w, 4.0, (o.width - cat_w - 8.0).max(10.0), (o.height - 8.0).max(10.0));
    let n = rows.len();
    let band = plot.h / n as f32;
    let bh = band * 0.78;
    let cx = plot.x + plot.w / 2.0;
    let has_hl = d.highlight.is_some();
    let mut prev: Option<(f32, f32)> = None;
    let one_series = o.props.color_opt("dataColors.default").unwrap_or(o.theme.series_color(0));
    for (i, &r) in rows.iter().enumerate() {
        let v = d.num(r, vi).unwrap_or(0.0).max(0.0);
        let w = (plot.w * (v / max) as f32).max(2.0);
        let y = plot.y + band * i as f32 + (band - bh) / 2.0;
        let rect = Rect::new(cx - w / 2.0, y, w, bh);
        // Connector from the previous bar.
        if let Some((pw, pb)) = prev {
            s.push(Shape::Path { points: vec![(cx - pw / 2.0, pb), (cx + pw / 2.0, pb), (cx + w / 2.0, y), (cx - w / 2.0, y)], closed: true, fill: Some(one_series.with_alpha(28)), stroke: None });
        }
        prev = Some((w, y + bh));
        let color = data_alpha(o, o.props.point_colors.get(&color_key(d, r)).copied().unwrap_or(one_series));
        let h = hit(d, r, POINT_ROLES);
        let dimmed = dim(o, &h.point);
        if has_hl {
            s.push(Shape::Rect { rect, fill: Some(color.with_alpha(70)), stroke: None, radius: 2.0 });
            if let Some(hv) = hl_num(d, r, vi) {
                let hw = (plot.w * (hv.max(0.0) / max) as f32).min(w);
                s.push_hit(Shape::Rect { rect: Rect::new(cx - hw / 2.0, y, hw, bh), fill: Some(color), stroke: None, radius: 2.0 }, h.clone(), false);
            }
            s.push_hit(Shape::Rect { rect, fill: None, stroke: None, radius: 0.0 }, h, false);
        } else {
            s.push_hit(Shape::Rect { rect, fill: Some(color), stroke: None, radius: 2.0 }, h, dimmed);
        }
        let pct = if first != 0.0 { format!("{:.0}%", v / first * 100.0) } else { String::new() };
        let vt = fmt.short(v);
        let tw = text_width(&vt, size);
        let lc = o.props.color_opt("labels.color");
        if tw + 8.0 < w {
            s.text(cx, y + bh / 2.0, vt, size, lc.unwrap_or(color.contrast_text()), Align::Center, Align::Center);
        } else {
            s.text(cx + w / 2.0 + 4.0, y + bh / 2.0, vt, size, lc.unwrap_or(o.theme.foreground), Align::Start, Align::Center);
        }
        if let Some(c) = ax {
            s.text(cat_w - 8.0, y + bh / 2.0 - if i > 0 { size * 0.45 } else { 0.0 }, ell(&cat_text(d, r, c), size + 1.0, cat_w - 10.0), size + 1.0, o.theme.foreground_muted, Align::End, Align::Center);
            if i > 0 {
                s.text(cat_w - 8.0, y + bh / 2.0 + size * 0.75, pct, size * 0.85, o.theme.foreground_muted.with_alpha(180), Align::End, Align::Center);
            }
        }
    }
    s
}
