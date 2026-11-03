//! Space-filling charts: squarified treemap (with details nesting), packed bubbles (circle
//! packing) and word cloud (spiral placement).

use std::collections::HashMap;

use super::RenderOpts;
use super::axis::*;
use super::common::*;
use super::scatter::overlaps;
use crate::color::Color;
use crate::doc::*;
use crate::scene::{Align, Line, Rect, Scene, Shape, text_width};
use crate::vquery::VisualData;

/// Squarified treemap layout: rectangles for `values` (any order) inside `r`.
pub fn squarify(values: &[f64], r: Rect) -> Vec<Rect> {
    let mut out = vec![Rect::default(); values.len()];
    let total: f64 = values.iter().map(|v| v.max(0.0)).sum();
    if total <= 0.0 || r.w <= 0.0 || r.h <= 0.0 {
        return out;
    }
    let mut idx: Vec<usize> = (0..values.len()).filter(|i| values.get(*i).is_some_and(|v| *v > 0.0)).collect();
    idx.sort_by(|a, b| values.get(*b).partial_cmp(&values.get(*a)).unwrap_or(std::cmp::Ordering::Equal));
    let scale = (r.w as f64 * r.h as f64) / total;
    let areas: Vec<(usize, f64)> = idx.iter().map(|i| (*i, values.get(*i).copied().unwrap_or(0.0) * scale)).collect();
    let mut rest = r;
    let mut i = 0;
    while i < areas.len() {
        let short = rest.w.min(rest.h) as f64;
        let mut row: Vec<(usize, f64)> = vec![];
        let mut best = f64::INFINITY;
        while i < areas.len() {
            let Some(next) = areas.get(i).copied() else { break };
            let mut cand = row.clone();
            cand.push(next);
            let w = worst(&cand, short);
            if row.is_empty() || w <= best {
                row = cand;
                best = w;
                i += 1;
            } else {
                break;
            }
        }
        let sum: f64 = row.iter().map(|x| x.1).sum();
        if rest.w >= rest.h {
            // Column on the left.
            let cw = (sum / rest.h as f64) as f32;
            let mut y = rest.y;
            for (k, a) in &row {
                let h = (a / cw as f64) as f32;
                if let Some(slot) = out.get_mut(*k) {
                    *slot = Rect::new(rest.x, y, cw, h);
                }
                y += h;
            }
            rest = Rect::new(rest.x + cw, rest.y, (rest.w - cw).max(0.0), rest.h);
        } else {
            let rh = (sum / rest.w as f64) as f32;
            let mut x = rest.x;
            for (k, a) in &row {
                let w = (a / rh as f64) as f32;
                if let Some(slot) = out.get_mut(*k) {
                    *slot = Rect::new(x, rest.y, w, rh);
                }
                x += w;
            }
            rest = Rect::new(rest.x, rest.y + rh, rest.w, (rest.h - rh).max(0.0));
        }
    }
    out
}

fn worst(row: &[(usize, f64)], short: f64) -> f64 {
    let sum: f64 = row.iter().map(|x| x.1).sum();
    let max = row.iter().map(|x| x.1).fold(0.0f64, f64::max);
    let min = row.iter().map(|x| x.1).fold(f64::INFINITY, f64::min);
    if sum <= 0.0 || min <= 0.0 {
        return f64::INFINITY;
    }
    let s2 = short * short;
    (s2 * max / (sum * sum)).max(sum * sum / (s2 * min))
}

pub fn render(_spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let Some(vi) = d.values().first().copied() else { return empty(o, "Add a field to Values") };
    let parent = d.first("axis").or_else(|| d.first("legend")).or_else(|| d.first("details")).filter(|c| d.cols.get(*c).is_some_and(|x| !x.is_value));
    let child = d.first("details").filter(|c| Some(*c) != parent && d.cols.get(*c).is_some_and(|x| !x.is_value));
    let rows: Vec<usize> = (0..d.rows.len()).filter(|r| d.num(*r, vi).is_some_and(|v| v.is_finite() && v > 0.0)).collect();
    if rows.is_empty() {
        return empty(o, "No data for the current filters");
    }
    let mut s = Scene::new(o.width, o.height);
    let area = Rect::new(0.0, 0.0, o.width, o.height);
    let fmt = ValueFmt::of_col(d, vi).with_props(o, "labels");
    let size = o.props.f32("labels.size", 9.0).max(9.0);
    let labels = o.props.bool("labels.show", true);
    let has_hl = d.highlight.is_some();
    let gapc = o.theme.visual_background;
    // Group by parent.
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    let mut gi: HashMap<String, usize> = HashMap::new();
    for &r in &rows {
        let k = parent.map(|p| d.cell(r, p).key()).unwrap_or_else(|| format!("{r}"));
        let i = *gi.entry(k.clone()).or_insert_with(|| {
            groups.push((k, vec![]));
            groups.len() - 1
        });
        if let Some(g) = groups.get_mut(i) {
            g.1.push(r);
        }
    }
    let gvals: Vec<f64> = groups.iter().map(|(_, rs)| rs.iter().filter_map(|r| d.num(*r, vi)).sum()).collect();
    let grects = squarify(&gvals, area);
    let draw_leaf = |s: &mut Scene, r: usize, rect: Rect, color: Color, title: String| {
        let inset = Rect::new(rect.x + 1.0, rect.y + 1.0, (rect.w - 2.0).max(0.0), (rect.h - 2.0).max(0.0));
        let h = hit(d, r, POINT_ROLES);
        let dimmed = dim(o, &h.point);
        if has_hl {
            s.push(Shape::Rect { rect: inset, fill: Some(color.with_alpha(70)), stroke: None, radius: 2.0 });
            let v = d.num(r, vi).unwrap_or(0.0);
            if let Some(hv) = hl_num(d, r, vi).filter(|x| *x > 0.0 && v > 0.0) {
                let hh = inset.h * (hv / v).clamp(0.0, 1.0) as f32;
                s.push(Shape::Rect { rect: Rect::new(inset.x, inset.bottom() - hh, inset.w, hh), fill: Some(color), stroke: None, radius: 2.0 });
            }
            s.push_hit(Shape::Rect { rect: inset, fill: None, stroke: None, radius: 0.0 }, h, false);
        } else {
            s.push_hit(Shape::Rect { rect: inset, fill: Some(color), stroke: Some(Line::solid(1.0, gapc)), radius: 2.0 }, h, dimmed);
        }
        if labels && inset.w > 30.0 && inset.h > size * 1.6 {
            let tc = label_color(o, Some(color));
            s.text(inset.x + 5.0, inset.y + 4.0, ell(&title, size + 1.0, inset.w - 10.0), size + 1.0, tc, Align::Start, Align::Start);
            if inset.h > size * 3.2 {
                s.text(inset.x + 5.0, inset.y + 6.0 + size * 1.3, ell(&fmt.short(d.num(r, vi).unwrap_or(0.0)), size, inset.w - 10.0), size, tc.with_alpha(210), Align::Start, Align::Start);
            }
        }
    };
    for (g, ((_, rs), rect)) in groups.iter().zip(grects.iter()).enumerate() {
        let first = rs.first().copied().unwrap_or(0);
        let base = data_alpha(o, o.props.point_colors.get(&color_key(d, first)).copied().filter(|_| child.is_none()).unwrap_or_else(|| series_color(o, g, &parent.map(|p| crate::fx::point_key(&d.identity(first, &[d.cols.get(p).map(|c| c.role.as_str()).unwrap_or("axis")]))).unwrap_or_default())));
        match child {
            Some(cc) if rect.w > 24.0 && rect.h > 30.0 => {
                let head = if rect.h > 46.0 && rect.w > 40.0 { size * 1.9 } else { 0.0 };
                let ptitle = parent.map(|p| cat_text(d, first, p)).unwrap_or_default();
                s.push(Shape::Rect { rect: Rect::new(rect.x + 0.5, rect.y + 0.5, rect.w - 1.0, rect.h - 1.0), fill: Some(base.shade(0.82)), stroke: Some(Line::solid(1.5, gapc)), radius: 3.0 });
                if head > 0.0 {
                    s.text_bold(rect.x + 6.0, rect.y + head / 2.0 + 1.0, ell(&ptitle, size + 1.0, rect.w - 12.0), size + 1.0, base.shade(-0.45), Align::Start, Align::Center);
                }
                let inner = Rect::new(rect.x + 2.0, rect.y + head + 1.0, (rect.w - 4.0).max(0.0), (rect.h - head - 3.0).max(0.0));
                let cvals: Vec<f64> = rs.iter().map(|r| d.num(*r, vi).unwrap_or(0.0)).collect();
                let crects = squarify(&cvals, inner);
                let n = rs.len().max(1) as f64;
                for (k, (r, cr)) in rs.iter().zip(crects.iter()).enumerate() {
                    let c = o.props.point_colors.get(&color_key(d, *r)).copied().unwrap_or_else(|| base.shade(0.38 * k as f64 / n));
                    draw_leaf(&mut s, *r, *cr, c, cat_text(d, *r, cc));
                }
            }
            _ => {
                let title = parent.map(|p| cat_text(d, first, p)).unwrap_or_default();
                draw_leaf(&mut s, first, *rect, base, title);
            }
        }
    }
    s
}

pub fn packed_bubbles(_spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let Some(vi) = d.values().first().copied() else { return empty(o, "Add a field to Size") };
    let cat = d.first("axis").or_else(|| d.first("details"));
    let legend_col = d.first("legend").filter(|c| d.cols.get(*c).is_some_and(|x| !x.is_value));
    let mut rows: Vec<usize> = (0..d.rows.len()).filter(|r| d.num(*r, vi).is_some_and(|v| v.is_finite() && v > 0.0)).collect();
    if rows.is_empty() {
        return empty(o, "No data for the current filters");
    }
    rows.sort_by(|a, b| d.num(*b, vi).partial_cmp(&d.num(*a, vi)).unwrap_or(std::cmp::Ordering::Equal));
    rows.truncate(300);
    let mut s = Scene::new(o.width, o.height);
    let mut area = Rect::new(2.0, 2.0, o.width - 4.0, o.height - 4.0);
    let lvals = legend_col.map(|c| d.distinct(c)).unwrap_or_default();
    if let Some(lc) = legend_col {
        let items: Vec<LegendItem> = lvals
            .iter()
            .enumerate()
            .map(|(i, v)| LegendItem { label: rows.iter().find(|r| d.cell(**r, lc).key() == v.key()).map(|r| cat_text(d, *r, lc)).unwrap_or_else(|| v.text()), color: o.theme.series_color(i), line: false, hit: None, dimmed: false })
            .collect();
        area = legend(&mut s, o, &items, &d.cols.get(lc).map(|c| c.label.clone()).unwrap_or_default(), area);
    }
    let max = rows.first().and_then(|r| d.num(*r, vi)).unwrap_or(1.0).max(1e-12);
    let radii: Vec<f64> = rows.iter().map(|r| (d.num(*r, vi).unwrap_or(0.0) / max).sqrt() * 100.0).collect();
    let pos = pack(&radii);
    // Fit into the area.
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for ((x, y), r) in pos.iter().zip(radii.iter()) {
        x0 = x0.min(x - r);
        y0 = y0.min(y - r);
        x1 = x1.max(x + r);
        y1 = y1.max(y + r);
    }
    let k = ((area.w as f64 / (x1 - x0).max(1e-9)).min(area.h as f64 / (y1 - y0).max(1e-9))) as f32;
    let (ox, oy) = (area.x + area.w / 2.0 - k * ((x0 + x1) / 2.0) as f32, area.y + area.h / 2.0 - k * ((y0 + y1) / 2.0) as f32);
    let fmt = ValueFmt::of_col(d, vi).with_props(o, "labels");
    for (i, ((r, (x, y)), rad)) in rows.iter().zip(pos.iter()).zip(radii.iter()).enumerate() {
        let (cx, cy, rr) = (ox + k * *x as f32, oy + k * *y as f32, (k * *rad as f32 - 1.0).max(1.0));
        let color = match legend_col {
            Some(lc) => o.theme.series_color(lvals.iter().position(|v| v.key() == d.cell(*r, lc).key()).unwrap_or(0)),
            None => o.theme.series_color(i % o.theme.data_colors.len().max(1)),
        };
        let color = data_alpha(o, o.props.point_colors.get(&color_key(d, *r)).copied().unwrap_or(color));
        let h = hit(d, *r, POINT_ROLES);
        let dimmed = dim(o, &h.point);
        s.push_hit(Shape::Circle { cx, cy, r: rr, fill: Some(color), stroke: None }, h, dimmed);
        if let Some(c) = cat {
            let name = cat_text(d, *r, c);
            let fs = (rr * 0.32).clamp(7.0, 16.0);
            if rr > 16.0 && text_width(&name, fs) < rr * 1.9 {
                s.text(cx, cy - if rr > 26.0 { fs * 0.45 } else { 0.0 }, &name, fs, color.contrast_text(), Align::Center, Align::Center);
                if rr > 26.0 {
                    s.text(cx, cy + fs * 0.75, fmt.short(d.num(*r, vi).unwrap_or(0.0)), fs * 0.85, color.contrast_text().with_alpha(200), Align::Center, Align::Center);
                }
            } else if rr > 12.0 {
                s.text(cx, cy, ell(&name, 8.0, rr * 1.8), 8.0, color.contrast_text(), Align::Center, Align::Center);
            }
        }
    }
    s
}

/// Greedy circle packing: each circle goes to the tangent position closest to the centre.
pub fn pack(radii: &[f64]) -> Vec<(f64, f64)> {
    let mut pos: Vec<(f64, f64)> = Vec::with_capacity(radii.len());
    let gap = radii.iter().cloned().fold(0.0f64, f64::max) * 0.03;
    for (i, r) in radii.iter().enumerate() {
        if i == 0 {
            pos.push((0.0, 0.0));
            continue;
        }
        let mut best: Option<(f64, (f64, f64))> = None;
        for (j, (px, py)) in pos.iter().enumerate() {
            let rj = radii.get(j).copied().unwrap_or(0.0);
            for a in 0..36 {
                let ang = a as f64 / 36.0 * std::f64::consts::TAU;
                let dist = rj + r + gap;
                let c = (px + dist * ang.cos(), py + dist * ang.sin());
                let d2 = c.0 * c.0 + c.1 * c.1 * 1.6;
                if best.as_ref().is_some_and(|b| b.0 <= d2) {
                    continue;
                }
                let ok = pos.iter().enumerate().all(|(m, q)| {
                    let rm = radii.get(m).copied().unwrap_or(0.0);
                    ((c.0 - q.0).powi(2) + (c.1 - q.1).powi(2)).sqrt() >= rm + r + gap * 0.99
                });
                if ok {
                    best = Some((d2, c));
                }
            }
        }
        pos.push(best.map(|b| b.1).unwrap_or((0.0, 0.0)));
    }
    pos
}

pub fn word_cloud(_spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let Some(cat) = d.first("axis").or_else(|| d.first("details")).filter(|c| d.cols.get(*c).is_some_and(|x| !x.is_value)) else { return empty(o, "Add a field to Category") };
    let vi = d.values().first().copied();
    let legend_col = d.first("legend").filter(|c| d.cols.get(*c).is_some_and(|x| !x.is_value));
    let mut rows: Vec<usize> = (0..d.rows.len()).collect();
    let val = |r: usize| vi.and_then(|v| d.num(r, v)).filter(|v| v.is_finite()).unwrap_or(1.0).max(0.0);
    rows.sort_by(|a, b| val(*b).partial_cmp(&val(*a)).unwrap_or(std::cmp::Ordering::Equal));
    rows.truncate(150);
    if rows.is_empty() {
        return empty(o, "No data for the current filters");
    }
    let mut s = Scene::new(o.width, o.height);
    let max = rows.first().map(|r| val(*r)).unwrap_or(1.0).max(1e-12);
    let min = rows.last().map(|r| val(*r)).unwrap_or(0.0);
    let big = (o.height / 5.5).clamp(14.0, 56.0);
    let small = 9.0f32;
    let lvals = legend_col.map(|c| d.distinct(c)).unwrap_or_default();
    let mut placed: Vec<Rect> = Vec::new();
    let (cx, cy) = (o.width / 2.0, o.height / 2.0);
    for (i, r) in rows.iter().enumerate() {
        let t = ((val(*r) - min) / (max - min).max(1e-12)).sqrt() as f32;
        let size = small + (big - small) * t;
        let word = cat_text(d, *r, cat);
        let w = text_width(&word, size) * 1.05;
        let h = size * 1.1;
        let mut spot = None;
        for step in 0..1400 {
            let a = step as f32 * 0.35;
            let rad = 1.6 * a;
            let x = cx + rad * a.cos() * (o.width / o.height.max(1.0)).clamp(1.0, 3.0) - w / 2.0;
            let y = cy + rad * a.sin() - h / 2.0;
            let rect = Rect::new(x, y, w, h);
            if rect.x < 2.0 || rect.y < 2.0 || rect.right() > o.width - 2.0 || rect.bottom() > o.height - 2.0 {
                if rad > o.width.max(o.height) {
                    break;
                }
                continue;
            }
            if !placed.iter().any(|p| overlaps(p, &rect)) {
                spot = Some(rect);
                break;
            }
        }
        let Some(rect) = spot else { continue };
        placed.push(rect);
        let color = match legend_col {
            Some(lc) => o.theme.series_color(lvals.iter().position(|v| v.key() == d.cell(*r, lc).key()).unwrap_or(0)),
            None => o.theme.series_color(i % o.theme.data_colors.len().clamp(1, 6)),
        };
        let color = o.props.point_colors.get(&color_key(d, *r)).copied().unwrap_or(color);
        let h = hit(d, *r, POINT_ROLES);
        let dimmed = dim(o, &h.point);
        s.push_hit(Shape::Text { x: rect.x + rect.w / 2.0, y: rect.y + rect.h / 2.0, text: word, size, color, bold: t > 0.45, italic: false, h: Align::Center, v: Align::Center, angle: 0.0 }, h, dimmed);
    }
    s
}
