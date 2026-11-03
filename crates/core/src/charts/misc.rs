//! Other visuals: Q&A, Python/R script images, box plot, bullet graph, Gantt, radar and
//! sankey.

use std::collections::HashMap;

use super::RenderOpts;
use super::axis::*;
use super::common::*;
use crate::color::Color;
use crate::doc::*;
use crate::scalar::Scalar;
use crate::scene::{Align, Hit, Line, Rect, Scene, Shape, text_width};
use crate::vquery::VisualData;

pub fn qna(spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let mut s = Scene::new(o.width, o.height);
    let bh = 30.0f32.min(o.height);
    let r = Rect::new(1.0, 1.0, o.width - 2.0, bh - 2.0);
    s.push(Shape::Rect { rect: r, fill: Some(o.theme.visual_background), stroke: Some(Line::solid(1.2, o.theme.accent.with_alpha(160))), radius: bh / 2.0 });
    let q = spec.question.trim();
    if q.is_empty() {
        s.text(14.0, bh / 2.0, "Ask a question about your data", 11.0, o.theme.foreground_muted, Align::Start, Align::Center);
    } else {
        s.text(14.0, bh / 2.0, ell(q, 11.0, o.width - 28.0), 11.0, o.theme.foreground, Align::Start, Align::Center);
    }
    s.actions.push((r, "qna:input".into()));
    if q.is_empty() || o.height < bh + 40.0 {
        if q.is_empty() {
            for (i, t) in ["total sales by region", "top 5 products by profit", "average price per category"].iter().enumerate() {
                s.text(14.0, bh + 18.0 + i as f32 * 18.0, format!("Try: \u{201C}{t}\u{201D}"), 10.0, o.theme.foreground_muted, Align::Start, Align::Center);
            }
        }
        return s;
    }
    let mut inner = RenderOpts { width: o.width, height: o.height - bh - 8.0, ..*o };
    inner.analytics = &[];
    let kind = if d.first("axis").is_some() { VisualKind::ClusteredColumn } else { VisualKind::Card };
    let sub = super::render(&VisualSpec { kind, ..VisualSpec::default() }, d, &inner);
    embed(&mut s, sub, 0.0, bh + 8.0);
    s
}

/// Width and height of a base64 PNG (from its IHDR chunk).
pub fn png_size(b64: &str) -> Option<(u32, u32)> {
    let alphabet = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a') as u32 + 26,
            b'0'..=b'9' => (c - b'0') as u32 + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        })
    };
    let mut bytes = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0;
    for c in b64.bytes().filter(|c| !c.is_ascii_whitespace()).take(40) {
        let Some(v) = alphabet(c) else { break };
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    let be = |i: usize| -> Option<u32> { Some(u32::from_be_bytes([*bytes.get(i)?, *bytes.get(i + 1)?, *bytes.get(i + 2)?, *bytes.get(i + 3)?])) };
    if bytes.get(1..4) != Some(b"PNG") {
        return None;
    }
    Some((be(16)?, be(20)?))
}

pub fn script(spec: &VisualSpec, _d: &VisualData, o: &RenderOpts) -> Scene {
    let mut s = Scene::new(o.width, o.height);
    match o.script_image {
        Some(img) if !img.is_empty() => {
            let (w, h) = png_size(img).map(|(w, h)| (w as f32, h as f32)).unwrap_or((o.width, o.height));
            let k = (o.width / w.max(1.0)).min(o.height / h.max(1.0));
            let (iw, ih) = (w * k, h * k);
            s.push(Shape::Image { rect: Rect::new((o.width - iw) / 2.0, (o.height - ih) / 2.0, iw, ih), png_base64: img.to_string() });
        }
        _ => {
            let lang = if spec.kind == VisualKind::RVisual { "R" } else { "Python" };
            let need = if spec.kind == VisualKind::RVisual { "needs Rscript" } else { "needs python3 with matplotlib" };
            let cx = o.width / 2.0;
            let cy = o.height / 2.0;
            s.push(Shape::Rect { rect: Rect::new(cx - 18.0, cy - 40.0, 36.0, 28.0), fill: None, stroke: Some(Line::solid(1.5, o.theme.foreground_muted.with_alpha(140))), radius: 4.0 });
            s.text(cx, cy - 26.0, if lang == "R" { "R" } else { "Py" }, 11.0, o.theme.foreground_muted, Align::Center, Align::Center);
            s.text(cx, cy + 2.0, ell("Run the script (Format → Script)", 11.0, o.width - 12.0), 11.0, o.theme.foreground, Align::Center, Align::Center);
            s.text(cx, cy + 20.0, ell(&format!("{lang} visual — {need}"), 9.5, o.width - 12.0), 9.5, o.theme.foreground_muted, Align::Center, Align::Center);
        }
    }
    s
}

pub fn box_plot(_spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let Some(vi) = d.values().first().copied() else { return empty(o, "Add a field to Value") };
    let ax = d.first("axis");
    let mut groups: Vec<(String, Vec<f64>, usize)> = Vec::new();
    let mut gi: HashMap<String, usize> = HashMap::new();
    for r in 0..d.rows.len() {
        let Some(v) = d.num(r, vi).filter(|v| v.is_finite()) else { continue };
        let k = ax.map(|c| d.cell(r, c).key()).unwrap_or_default();
        let i = *gi.entry(k).or_insert_with(|| {
            groups.push((ax.map(|c| cat_text(d, r, c)).unwrap_or_default(), vec![], r));
            groups.len() - 1
        });
        if let Some(g) = groups.get_mut(i) {
            g.1.push(v);
        }
    }
    if groups.is_empty() {
        return empty(o, "No data for the current filters");
    }
    let mut s = Scene::new(o.width, o.height);
    let all: Vec<f64> = groups.iter().flat_map(|g| g.1.iter().copied()).collect();
    let lo = all.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = all.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let fmt = ValueFmt::of_col(d, vi).with_props(o, "yAxis");
    let sc = Scale::from_props(o, "yAxis", lo, hi, lo >= 0.0 && lo < hi * 0.3, tick_target(o.height * 0.8));
    let yw = value_axis_width(o, &sc, &fmt, "yAxis");
    let labels: Vec<String> = groups.iter().map(|g| g.0.clone()).collect();
    let pw = o.width - yw - 8.0;
    let band = pw / groups.len() as f32;
    let lay = cat_layout(&labels, band, 9.5, o.height * 0.25);
    let plot = Rect::new(yw, 8.0, pw, (o.height - lay.extent - 10.0).max(10.0));
    value_axis_v(&mut s, o, &sc, plot, &fmt, "yAxis", false, true);
    let y = |v: f64| plot.bottom() - plot.h * sc.frac(v);
    for (i, (label, vals, row)) in groups.iter().enumerate() {
        let q1 = crate::analytics::percentile(vals, 25.0);
        let q2 = crate::analytics::percentile(vals, 50.0);
        let q3 = crate::analytics::percentile(vals, 75.0);
        let iqr = q3 - q1;
        let wlo = vals.iter().cloned().filter(|v| *v >= q1 - 1.5 * iqr).fold(f64::INFINITY, f64::min);
        let whi = vals.iter().cloned().filter(|v| *v <= q3 + 1.5 * iqr).fold(f64::NEG_INFINITY, f64::max);
        let mean = crate::analytics::mean(vals);
        let c = data_alpha(o, series_color(o, if o.props.bool("dataColors.showAll", false) { i } else { 0 }, &color_key(d, *row)));
        let cx = plot.x + band * (i as f32 + 0.5);
        let bw = (band * 0.45).min(60.0);
        let st = Line::solid(1.4, c.shade(-0.25));
        if wlo.is_finite() && whi.is_finite() {
            s.line((cx, y(whi)), (cx, y(q3)), st.clone());
            s.line((cx, y(q1)), (cx, y(wlo)), st.clone());
            s.line((cx - bw * 0.25, y(whi)), (cx + bw * 0.25, y(whi)), st.clone());
            s.line((cx - bw * 0.25, y(wlo)), (cx + bw * 0.25, y(wlo)), st.clone());
        }
        let rect = Rect::new(cx - bw / 2.0, y(q3), bw, (y(q1) - y(q3)).max(1.0));
        let h = Hit { point: d.identity(*row, &["axis"]), tooltip: vec![(label.clone(), String::new()), ("Max".into(), fmt.full(whi)), ("Q3".into(), fmt.full(q3)), ("Median".into(), fmt.full(q2)), ("Q1".into(), fmt.full(q1)), ("Min".into(), fmt.full(wlo)), ("Mean".into(), fmt.full(mean)), ("Count".into(), vals.len().to_string())] };
        let dimmed = dim(o, &h.point);
        s.push_hit(Shape::Rect { rect, fill: Some(c.with_alpha(70)), stroke: Some(st.clone()), radius: 2.0 }, h, dimmed);
        s.line((rect.x, y(q2)), (rect.right(), y(q2)), Line::solid(2.0, c.shade(-0.3)));
        let (mx, my) = (cx, y(mean));
        s.line((mx - 3.0, my - 3.0), (mx + 3.0, my + 3.0), Line::solid(1.2, c.shade(-0.3)));
        s.line((mx - 3.0, my + 3.0), (mx + 3.0, my - 3.0), Line::solid(1.2, c.shade(-0.3)));
        for v in vals.iter().filter(|v| **v < q1 - 1.5 * iqr || **v > q3 + 1.5 * iqr) {
            s.push(Shape::Circle { cx, cy: y(*v), r: 2.5, fill: None, stroke: Some(Line::solid(1.2, c)) });
        }
        if i % lay.step.max(1) == 0 {
            draw_cat_label(&mut s, &lay, cx, plot.bottom(), label, axis_text(o, "xAxis"));
        }
    }
    s
}

pub fn bullet(_spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let Some(vi) = d.first("values").filter(|c| d.cols.get(*c).is_some_and(|x| x.is_value)).or_else(|| d.values().first().copied()) else { return empty(o, "Add a field to Value") };
    let ti = d.first("target").filter(|c| d.cols.get(*c).is_some_and(|x| x.is_value));
    let ax = d.first("axis");
    let rows: Vec<usize> = (0..d.rows.len()).collect();
    if rows.is_empty() {
        return empty(o, "No data for the current filters");
    }
    let mut s = Scene::new(o.width, o.height);
    let fmt = ValueFmt::of_col(d, vi).with_props(o, "labels");
    let max = rows.iter().map(|r| d.num(*r, vi).unwrap_or(0.0).max(ti.and_then(|t| d.num(*r, t)).unwrap_or(0.0) * 1.15)).fold(0.0f64, f64::max).max(1e-12);
    let size = 9.5;
    let label_w = ax.map(|c| rows.iter().map(|r| text_width(&cat_text(d, *r, c), size)).fold(0.0, f32::max).min(o.width * 0.3) + 12.0).unwrap_or(4.0);
    let sc = Scale::linear(0.0, max, true, tick_target(o.width - label_w));
    let plot = Rect::new(label_w, 2.0, (o.width - label_w - 46.0).max(10.0), (o.height - 20.0).max(10.0));
    let band = plot.h / rows.len() as f32;
    let px = |v: f64| plot.x + plot.w * sc.frac(v);
    let dark = o.theme.visual_background.luminance() < 0.4;
    let shades = if dark { [Color::rgb(0x33, 0x3B, 0x4C), Color::rgb(0x2A, 0x31, 0x40), Color::rgb(0x22, 0x28, 0x35)] } else { [Color::rgb(0xD5, 0xD9, 0xE0), Color::rgb(0xE3, 0xE6, 0xEB), Color::rgb(0xF0, 0xF2, 0xF5)] };
    let accent = o.props.color_opt("dataColors.default").unwrap_or(o.theme.series_color(0));
    for (i, r) in rows.iter().enumerate() {
        let y = plot.y + band * i as f32;
        let bh = (band * 0.62).min(30.0);
        let yc = y + band / 2.0;
        let v = d.num(*r, vi).unwrap_or(0.0);
        let t = ti.and_then(|t| d.num(*r, t));
        let reference = t.unwrap_or(max / 1.15);
        let bands = [0.6, 0.9, 1.15];
        let mut x0 = 0.0;
        for (k, f) in bands.iter().enumerate() {
            let x1 = (reference * f).min(sc.hi);
            s.rect(Rect::new(px(x0), yc - bh / 2.0, (px(x1) - px(x0)).max(0.0), bh), *shades.get(k).unwrap_or(&shades[0]));
            x0 = x1;
        }
        let c = match t {
            Some(t) if v < t => o.theme.bad.lerp(accent, 0.25),
            _ => accent,
        };
        let h = hit(d, *r, POINT_ROLES);
        let dimmed = dim(o, &h.point);
        s.push_hit(Shape::Rect { rect: Rect::new(px(0.0), yc - bh * 0.2, (px(v) - px(0.0)).max(1.0), bh * 0.4), fill: Some(c), stroke: None, radius: 1.5 }, h, dimmed);
        if let Some(t) = t {
            s.line((px(t), yc - bh * 0.42), (px(t), yc + bh * 0.42), Line::solid(2.5, o.theme.foreground));
        }
        if let Some(c) = ax {
            s.text(label_w - 6.0, yc, ell(&cat_text(d, *r, c), size, label_w - 8.0), size, o.theme.foreground_muted, Align::End, Align::Center);
        }
        s.text(plot.right() + 4.0, yc, fmt.short(v), size, o.theme.foreground, Align::Start, Align::Center);
    }
    let afmt = ValueFmt::of_col(d, vi);
    for t in &sc.ticks {
        s.text(px(*t), plot.bottom() + 4.0, afmt.short(*t), 8.5, o.theme.foreground_muted, Align::Center, Align::Start);
    }
    s
}

pub fn gantt(_spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let ax = d.first("axis");
    let st = d.first("start");
    let du = d.first("values").filter(|c| d.cols.get(*c).is_some_and(|x| x.is_value));
    let (Some(st), Some(du)) = (st, du) else { return empty(o, "Add fields to Task, Start and Duration") };
    let rows: Vec<usize> = (0..d.rows.len()).filter(|r| d.num(*r, st).is_some_and(f64::is_finite)).collect();
    if rows.is_empty() {
        return empty(o, "Start must be a date or number");
    }
    let mut s = Scene::new(o.width, o.height);
    let legend_col = d.first("legend").filter(|c| d.cols.get(*c).is_some_and(|x| !x.is_value));
    let lvals = legend_col.map(|c| d.distinct(c)).unwrap_or_default();
    let mut area = Rect::new(2.0, 2.0, o.width - 4.0, o.height - 4.0);
    if let Some(lc) = legend_col {
        let items: Vec<LegendItem> = lvals.iter().enumerate().map(|(i, v)| LegendItem { label: v.text(), color: o.theme.series_color(i), line: false, hit: None, dimmed: false }).collect();
        area = legend(&mut s, o, &items, &d.cols.get(lc).map(|c| c.label.clone()).unwrap_or_default(), area);
    }
    let start = |r: usize| d.num(r, st).unwrap_or(0.0);
    let end = |r: usize| start(r) + d.num(r, du).unwrap_or(0.0).max(0.0);
    let lo = rows.iter().map(|r| start(*r)).fold(f64::INFINITY, f64::min);
    let hi = rows.iter().map(|r| end(*r)).fold(f64::NEG_INFINITY, f64::max).max(lo + 1.0);
    let temporal = d.cols.get(st).is_some_and(|c| c.col_type.is_temporal() || (lo > 20000.0 && hi < 80000.0));
    let size = 9.5;
    let label_w = ax.map(|c| rows.iter().map(|r| text_width(&cat_text(d, *r, c), size)).fold(0.0, f32::max).min(area.w * 0.3) + 12.0).unwrap_or(4.0);
    let plot = Rect::new(area.x + label_w, area.y + 20.0, (area.w - label_w - 6.0).max(10.0), (area.h - 22.0).max(10.0));
    let px = |v: f64| plot.x + plot.w * ((v - lo) / (hi - lo)) as f32;
    // Time axis on top.
    let ticks = nice_ticks(lo, hi, tick_target(plot.w));
    let gl = grid_line(o);
    let span = hi - lo;
    for t in ticks.iter().filter(|t| **t >= lo && **t <= hi) {
        let x = px(*t);
        s.line((x, plot.y), (x, plot.bottom()), gl.clone());
        let label = if temporal { crate::numfmt::format_date(*t, if span > 120.0 { "mmm yyyy" } else { "dd mmm" }) } else { short_num(*t, "Auto") };
        s.text(x, plot.y - 4.0, label, 8.5, o.theme.foreground_muted, Align::Center, Align::End);
    }
    let band = (plot.h / rows.len() as f32).min(30.0);
    for (i, r) in rows.iter().enumerate() {
        let y = plot.y + band * i as f32;
        if y + band > area.bottom() + 1.0 {
            break;
        }
        let c = match legend_col {
            Some(lc) => o.theme.series_color(lvals.iter().position(|v| v.key() == d.cell(*r, lc).key()).unwrap_or(0)),
            None => series_color(o, 0, &color_key(d, *r)),
        };
        let rect = Rect::new(px(start(*r)), y + band * 0.2, (px(end(*r)) - px(start(*r))).max(3.0), band * 0.6);
        let h = hit(d, *r, POINT_ROLES);
        let dimmed = dim(o, &h.point);
        s.push_hit(Shape::Rect { rect, fill: Some(data_alpha(o, c)), stroke: None, radius: 3.0 }, h, dimmed);
        if let Some(ac) = ax {
            s.text(plot.x - 6.0, y + band / 2.0, ell(&cat_text(d, *r, ac), size, label_w - 8.0), size, o.theme.foreground_muted, Align::End, Align::Center);
        }
    }
    s
}

pub fn radar(_spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let Some(ax) = d.first("axis") else { return empty(o, "Add a field to Category") };
    if d.values().is_empty() {
        return empty(o, "Add a field to Values");
    }
    let cf = super::cartesian::cfg(VisualKind::ClusteredColumn);
    let all: Vec<usize> = (0..d.rows.len()).collect();
    let p = super::cartesian::pivot(d, o, &all, &cf);
    let n = p.cats.len();
    if n < 3 {
        return super::cartesian::render(&VisualSpec { kind: VisualKind::ClusteredColumn, ..VisualSpec::default() }, d, o);
    }
    let _ = ax;
    let mut s = Scene::new(o.width, o.height);
    let mut area = Rect::new(2.0, 2.0, o.width - 4.0, o.height - 4.0);
    if p.series.len() > 1 || d.first("legend").is_some() {
        let items: Vec<LegendItem> = p.series.iter().map(|x| LegendItem { label: x.name.clone(), color: x.color, line: false, hit: None, dimmed: false }).collect();
        area = legend(&mut s, o, &items, &p.legend_title, area);
    }
    let max = p.series.iter().flat_map(|x| x.values.iter().flatten()).fold(0.0f64, |a, b| a.max(*b)).max(1e-12);
    let ticks = nice_ticks(0.0, max, 4);
    let top = ticks.last().copied().unwrap_or(max).max(1e-12);
    let size = 9.0;
    let lw = p.cats.iter().map(|c| text_width(c, size)).fold(0.0, f32::max).min(area.w * 0.22);
    let r = ((area.w / 2.0 - lw - 10.0).min(area.h / 2.0 - size * 1.6)).max(10.0);
    let (cx, cy) = (area.x + area.w / 2.0, area.y + area.h / 2.0);
    let pt = |i: usize, f: f32| {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        (cx + r * f * a.sin(), cy - r * f * a.cos())
    };
    let gl = Line::solid(1.0, grid_default(o));
    for t in ticks.iter().skip(1) {
        let f = (*t / top) as f32;
        let ring: Vec<(f32, f32)> = (0..n).map(|i| pt(i, f)).collect();
        s.push(Shape::Path { points: ring, closed: true, fill: None, stroke: Some(gl.clone()) });
        s.text(cx + 3.0, cy - r * f, p.fmt.short(*t), 8.0, o.theme.foreground_muted, Align::Start, Align::Center);
    }
    for (i, c) in p.cats.iter().enumerate() {
        let e = pt(i, 1.0);
        s.line((cx, cy), e, gl.clone());
        let (lx, ly) = pt(i, 1.0 + 10.0 / r);
        let h = if (lx - cx).abs() < 4.0 { Align::Center } else if lx > cx { Align::Start } else { Align::End };
        s.text(lx, ly, ell(c, size, lw.max(30.0)), size, o.theme.foreground_muted, h, Align::Center);
    }
    for sr in &p.series {
        let pts: Vec<(f32, f32)> = (0..n).map(|i| pt(i, (sr.values.get(i).copied().flatten().unwrap_or(0.0).max(0.0) / top) as f32)).collect();
        s.push(Shape::Path { points: pts.clone(), closed: true, fill: Some(sr.color.with_alpha(45)), stroke: Some(Line::solid(2.0, sr.color)) });
        for (i, q) in pts.iter().enumerate() {
            if let Some(row) = sr.rows.get(i).copied().flatten() {
                let h = hit(d, row, POINT_ROLES);
                let dimmed = dim(o, &h.point);
                s.push_hit(Shape::Circle { cx: q.0, cy: q.1, r: 3.0, fill: Some(sr.color), stroke: Some(Line::solid(1.0, o.theme.visual_background)) }, h, dimmed);
            }
        }
    }
    s
}

pub fn sankey(_spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let (Some(sc), Some(dc)) = (d.first("source"), d.first("destination")) else { return empty(o, "Add fields to Source and Destination") };
    let vi = d.values().first().copied();
    let w = |r: usize| vi.and_then(|v| d.num(r, v)).filter(|v| v.is_finite()).unwrap_or(1.0).max(0.0);
    let rows: Vec<usize> = (0..d.rows.len()).filter(|r| w(*r) > 0.0).collect();
    if rows.is_empty() {
        return empty(o, "No data for the current filters");
    }
    let mut s = Scene::new(o.width, o.height);
    // Nodes per side, ordered by total.
    let side = |col: usize| -> Vec<(String, Scalar, f64)> {
        let mut m: Vec<(String, Scalar, f64)> = Vec::new();
        for r in &rows {
            let v = d.cell(*r, col);
            match m.iter_mut().find(|x| x.1.key() == v.key()) {
                Some(x) => x.2 += w(*r),
                None => m.push((cat_text(d, *r, col), v.clone(), w(*r))),
            }
        }
        m.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        m
    };
    let left = side(sc);
    let right = side(dc);
    let total: f64 = left.iter().map(|x| x.2).sum::<f64>().max(1e-12);
    let size = 9.5;
    let fmt = vi.map(|v| ValueFmt::of_col(d, v)).unwrap_or_default();
    let lw = left.iter().map(|x| text_width(&x.0, size)).fold(0.0, f32::max).min(o.width * 0.25) + 10.0;
    let rw = right.iter().map(|x| text_width(&x.0, size)).fold(0.0, f32::max).min(o.width * 0.25) + 10.0;
    let node_w = 12.0;
    let (x0, x1) = (lw, o.width - rw - node_w);
    let gap = 6.0;
    let k = (o.height - 4.0 - gap * (left.len().max(right.len()).saturating_sub(1)) as f32).max(10.0) / total as f32;
    let layout = |nodes: &[(String, Scalar, f64)]| -> Vec<(f32, f32)> {
        let used: f32 = nodes.iter().map(|n| (n.2 as f32 * k).max(2.0)).sum::<f32>() + gap * nodes.len().saturating_sub(1) as f32;
        let mut y = 2.0 + ((o.height - 4.0 - used) / 2.0).max(0.0);
        nodes.iter().map(|n| {
            let h = (n.2 as f32 * k).max(2.0);
            let out = (y, h);
            y += h + gap;
            out
        }).collect()
    };
    let lpos = layout(&left);
    let rpos = layout(&right);
    // Links.
    let mut lcur: Vec<f32> = lpos.iter().map(|p| p.0).collect();
    let mut rcur: Vec<f32> = rpos.iter().map(|p| p.0).collect();
    let mut links: Vec<(usize, usize, usize)> = rows.iter().filter_map(|r| {
        let li = left.iter().position(|x| x.1.key() == d.cell(*r, sc).key())?;
        let ri = right.iter().position(|x| x.1.key() == d.cell(*r, dc).key())?;
        Some((li, ri, *r))
    }).collect();
    links.sort_by_key(|l| (l.0, l.1));
    for (li, ri, r) in links {
        let h = w(r) as f32 * k;
        let (Some(ly), Some(ry)) = (lcur.get(li).copied(), rcur.get(ri).copied()) else { continue };
        if let Some(v) = lcur.get_mut(li) {
            *v += h;
        }
        if let Some(v) = rcur.get_mut(ri) {
            *v += h;
        }
        let color = o.theme.series_color(li);
        let (ax, bx) = (x0 + node_w, x1);
        let curve = |y0: f32, y1: f32| -> Vec<(f32, f32)> {
            (0..=16).map(|i| {
                let t = i as f32 / 16.0;
                let x = ax + (bx - ax) * t;
                let e = t * t * (3.0 - 2.0 * t);
                (x, y0 + (y1 - y0) * e)
            }).collect()
        };
        let mut poly = curve(ly, ry);
        let mut bottom = curve(ly + h, ry + h);
        bottom.reverse();
        poly.extend(bottom);
        let hh = hit(d, r, POINT_ROLES);
        let dimmed = dim(o, &hh.point);
        s.push_hit(Shape::Path { points: poly, closed: true, fill: Some(color.with_alpha(70)), stroke: None }, hh, dimmed);
    }
    for (i, (n, p)) in left.iter().zip(lpos.iter()).enumerate() {
        let c = o.theme.series_color(i);
        let pt = d.cols.get(sc).map(|x| vec![(x.field.clone(), n.1.clone())]).unwrap_or_default();
        s.push_hit(Shape::Rect { rect: Rect::new(x0, p.0, node_w, p.1), fill: Some(c), stroke: None, radius: 2.0 }, Hit { point: pt, tooltip: vec![(n.0.clone(), fmt.full(n.2))] }, false);
        s.text(x0 - 5.0, p.0 + p.1 / 2.0, ell(&n.0, size, lw - 8.0), size, o.theme.foreground, Align::End, Align::Center);
    }
    for (n, p) in right.iter().zip(rpos.iter()) {
        let c = o.theme.foreground_muted;
        let pt = d.cols.get(dc).map(|x| vec![(x.field.clone(), n.1.clone())]).unwrap_or_default();
        s.push_hit(Shape::Rect { rect: Rect::new(x1, p.0, node_w, p.1), fill: Some(c), stroke: None, radius: 2.0 }, Hit { point: pt, tooltip: vec![(n.0.clone(), fmt.full(n.2))] }, false);
        s.text(x1 + node_w + 5.0, p.0 + p.1 / 2.0, ell(&n.0, size, rw - 8.0), size, o.theme.foreground, Align::Start, Align::Center);
    }
    s
}
