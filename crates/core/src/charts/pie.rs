//! Pie and donut charts: outside labels with leader lines (de-overlapped per side), percent,
//! a centre total on donuts, cross-highlight as partial slices.

use super::RenderOpts;
use super::axis::*;
use super::common::*;
use crate::doc::*;
use crate::scene::{Align, Line, Rect, Scene, Shape, text_width};
use crate::vquery::VisualData;

pub fn render(spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let Some(vi) = d.values().first().copied() else { return empty(o, "Add a field to Values") };
    let cat = d.first("legend").or_else(|| d.first("details")).or_else(|| d.first("axis")).filter(|c| d.cols.get(*c).is_some_and(|x| !x.is_value));
    let rows: Vec<usize> = (0..d.rows.len()).filter(|r| d.num(*r, vi).is_some_and(|v| v.is_finite() && v > 0.0)).collect();
    let total: f64 = rows.iter().filter_map(|r| d.num(*r, vi)).sum();
    if rows.is_empty() || total <= 0.0 {
        return empty(o, "No data for the current filters");
    }
    let mut s = Scene::new(o.width, o.height);
    let label_of = |r: usize| cat.map(|c| cat_text(d, r, c)).unwrap_or_else(|| d.cols.get(vi).map(|c| c.label.clone()).unwrap_or_default());
    let colors: Vec<crate::color::Color> = rows.iter().enumerate().map(|(i, r)| data_alpha(o, series_color(o, i, &color_key(d, *r)))).collect();
    let labels_on = o.props.bool("labels.show", true);
    let content = o.props.text("labels.content", "Category, percent of total");
    let show_legend = o.props.bool("legend.show", true) && (!labels_on || !content.contains("Category") && content != "All detail labels");
    let mut area = Rect::new(2.0, 2.0, o.width - 4.0, o.height - 4.0);
    if show_legend {
        let items: Vec<LegendItem> = rows
            .iter()
            .zip(colors.iter())
            .map(|(r, c)| {
                let h = hit(d, *r, POINT_ROLES);
                let dimmed = dim(o, &h.point);
                LegendItem { label: label_of(*r), color: *c, line: false, hit: Some(h), dimmed }
            })
            .collect();
        let title = cat.and_then(|c| d.cols.get(c)).map(|c| c.label.clone()).unwrap_or_default();
        area = legend(&mut s, o, &items, &title, area);
    }
    let size = o.props.f32("labels.size", 9.0).max(8.0);
    let fmt = ValueFmt::of_col(d, vi).with_props(o, "labels");
    let texts: Vec<String> = rows
        .iter()
        .map(|r| {
            let v = d.num(*r, vi).unwrap_or(0.0);
            let pct = format!("{:.1}%", v / total * 100.0);
            let val = fmt.short(v);
            let cat = label_of(*r);
            let t = match content.as_str() {
                "Category" => cat.clone(),
                "Data value" => val.clone(),
                "Percent of total" => pct.clone(),
                "Category, data value" => format!("{cat}: {val}"),
                "All detail labels" => format!("{cat}: {val} ({pct})"),
                _ => format!("{cat}: {pct}"),
            };
            let tpl = o.props.text("labels.template", "");
            if tpl.trim().is_empty() { t } else { label_template(o, &val, &cat, "", &pct) }
        })
        .collect();
    let label_w = if labels_on { texts.iter().map(|t| text_width(t, size)).fold(0.0, f32::max).min(area.w * 0.28) + 22.0 } else { 0.0 };
    let (cx, cy) = (area.x + area.w / 2.0, area.y + area.h / 2.0);
    let r = ((area.w / 2.0 - label_w).min(area.h / 2.0 - if labels_on { 14.0 } else { 4.0 })).max(12.0);
    let inner = r * o.props.f32("donut.inner", if spec.kind == VisualKind::Donut { 55.0 } else { 0.0 }).clamp(0.0, 95.0) / 100.0;
    let gap = o.props.f32("slices.gap", 1.0);
    let rot = o.props.f32("slices.rotation", 0.0).to_radians();
    let stroke = if gap > 0.0 { Some(Line::solid(gap * 1.5, o.theme.visual_background)) } else { None };
    let mut a = rot;
    let has_hl = d.highlight.is_some();
    let mut mids = Vec::new();
    for (i, &row) in rows.iter().enumerate() {
        let v = d.num(row, vi).unwrap_or(0.0);
        let sweep = (v / total) as f32 * std::f32::consts::TAU;
        let color = colors.get(i).copied().unwrap_or(o.theme.accent);
        let h = hit(d, row, POINT_ROLES);
        let dimmed = dim(o, &h.point);
        if has_hl {
            s.push(Shape::Arc { cx, cy, r_inner: inner, r_outer: r, start: a, end: a + sweep, fill: color.with_alpha(70), stroke: stroke.clone() });
            if let Some(hv) = hl_num(d, row, vi).filter(|x| *x > 0.0) {
                let hs = sweep * (hv / v).clamp(0.0, 1.0) as f32;
                s.push(Shape::Arc { cx, cy, r_inner: inner, r_outer: r, start: a, end: a + hs, fill: color, stroke: stroke.clone() });
            }
            s.push_hit(Shape::Arc { cx, cy, r_inner: inner, r_outer: r, start: a, end: a + sweep, fill: crate::color::Color::TRANSPARENT, stroke: None }, h, false);
        } else {
            s.push_hit(Shape::Arc { cx, cy, r_inner: inner, r_outer: r, start: a, end: a + sweep, fill: color, stroke: stroke.clone() }, h, dimmed);
        }
        mids.push((a + sweep / 2.0, sweep, i));
        a += sweep;
    }
    // Centre total on donuts.
    if inner > 30.0 {
        let t = ValueFmt::of_col(d, vi).short(total);
        let ts = (inner * 0.42).clamp(10.0, 26.0);
        if text_width(&t, ts) < inner * 1.7 {
            s.text_bold(cx, cy - ts * 0.2, t, ts, o.theme.foreground, Align::Center, Align::Center);
            s.text(cx, cy + ts * 0.65, "Total", (ts * 0.45).max(8.0), o.theme.foreground_muted, Align::Center, Align::Center);
        }
    }
    if !labels_on {
        return s;
    }
    let pos = o.props.text("labels.position", "Prefer outside");
    if pos == "Inside" {
        for (mid, sweep, i) in &mids {
            let rr = if inner > 0.0 { (inner + r) / 2.0 } else { r * 0.62 };
            let (x, y) = (cx + rr * mid.sin(), cy - rr * mid.cos());
            let t = texts.get(*i).cloned().unwrap_or_default();
            if sweep * rr > text_width(&t, size).min(rr) && *sweep > 0.25 {
                let c = colors.get(*i).copied().unwrap_or(o.theme.accent);
                s.text(x, y, t, size, label_color(o, Some(c)), Align::Center, Align::Center);
            }
        }
        return s;
    }
    // Outside labels, de-overlapped per side.
    let line_h = size * 1.35;
    let mut sides: [Vec<(f32, f32, usize)>; 2] = [vec![], vec![]];
    for (mid, sweep, i) in &mids {
        if *sweep < 0.035 && mids.len() > 12 {
            continue;
        }
        let right = mid.sin() >= 0.0;
        let y = cy - (r + 12.0) * mid.cos();
        sides[if right { 1 } else { 0 }].push((y, *mid, *i));
    }
    let top = area.y + size * 0.6;
    let bottom = area.bottom() - size * 0.6;
    for (side_i, side) in sides.iter_mut().enumerate() {
        side.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        // Too many for the height: drop the smallest slices.
        let cap = (((bottom - top) / line_h).floor() as usize).max(1);
        if side.len() > cap {
            let mut by_size: Vec<(f32, f32, usize)> = side.clone();
            by_size.sort_by(|a, b| {
                let va = mids.iter().find(|m| m.2 == a.2).map(|m| m.1).unwrap_or(0.0);
                let vb = mids.iter().find(|m| m.2 == b.2).map(|m| m.1).unwrap_or(0.0);
                vb.partial_cmp(&va).unwrap_or(std::cmp::Ordering::Equal)
            });
            let keep: Vec<usize> = by_size.iter().take(cap).map(|x| x.2).collect();
            side.retain(|x| keep.contains(&x.2));
        }
        let mut ys: Vec<f32> = side.iter().map(|x| x.0.clamp(top, bottom)).collect();
        for k in 1..ys.len() {
            let prev = ys.get(k - 1).copied().unwrap_or(top);
            if let Some(y) = ys.get_mut(k)
                && *y < prev + line_h
            {
                *y = prev + line_h;
            }
        }
        if let Some(last) = ys.last().copied()
            && last > bottom
        {
            let shift = last - bottom;
            for y in ys.iter_mut() {
                *y -= shift;
            }
            for k in (0..ys.len().saturating_sub(1)).rev() {
                let next = ys.get(k + 1).copied().unwrap_or(bottom);
                if let Some(y) = ys.get_mut(k)
                    && *y > next - line_h
                {
                    *y = next - line_h;
                }
            }
        }
        let right = side_i == 1;
        for ((_, mid, i), ly) in side.iter().zip(ys.iter()) {
            let (ex, ey) = (cx + r * mid.sin(), cy - r * mid.cos());
            let (bx, by) = (cx + (r + 8.0) * mid.sin(), cy - (r + 8.0) * mid.cos());
            let lx = if right { cx + r + 14.0 } else { cx - r - 14.0 };
            let lc = o.theme.foreground_muted.with_alpha(150);
            let bx = if right { bx.min(lx - 2.0) } else { bx.max(lx + 2.0) };
            s.push(Shape::Path { points: vec![(ex, ey), (bx, by.clamp(*ly - line_h, *ly + line_h)), (lx, *ly)], closed: false, fill: None, stroke: Some(Line::solid(1.0, lc)) });
            let t = texts.get(*i).cloned().unwrap_or_default();
            let maxw = if right { area.right() - lx - 4.0 } else { lx - area.x - 4.0 };
            let col = o.props.color_opt("labels.color").unwrap_or(o.theme.foreground);
            if right {
                s.text(lx + 3.0, *ly, ell(&t, size, maxw), size, col, Align::Start, Align::Center);
            } else {
                s.text(lx - 3.0, *ly, ell(&t, size, maxw), size, col, Align::End, Align::Center);
            }
        }
    }
    s
}
