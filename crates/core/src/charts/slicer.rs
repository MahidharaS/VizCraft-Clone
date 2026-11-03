//! Slicers: vertical list (checkboxes, "Select all", search), tiles, dropdown, and numeric /
//! date ranges (between, ≤, ≥) with handles. Interactions go out as scene actions (see
//! `charts` module docs).

use super::RenderOpts;
use super::common::*;
use crate::color::Color;
use crate::doc::*;
use crate::scene::{Align, Line, Rect, Scene, Shape, text_width};
use crate::vquery::VisualData;

fn style(spec: &VisualSpec, o: &RenderOpts) -> String {
    let p = o.props.text("slicer.style", "Vertical list");
    if p != "Vertical list" {
        return p;
    }
    match spec.slicer_mode.as_str() {
        "Dropdown" => "Dropdown".into(),
        "Tile" => "Tile".into(),
        "Between" => "Between".into(),
        "Before" => "Less than or equal to".into(),
        "After" => "Greater than or equal to".into(),
        "Relative date" => "Relative date".into(),
        _ => "Vertical list".into(),
    }
}

fn action(v: &crate::scalar::Scalar) -> String {
    format!("slicer:{}", serde_json::to_string(v).unwrap_or_default())
}

pub fn render(spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let st = style(spec, o);
    if d.cols.is_empty() {
        return empty(o, "Add a field to the slicer");
    }
    match st.as_str() {
        "Between" | "Less than or equal to" | "Greater than or equal to" => range(spec, d, o, &st),
        "Tile" => tiles(spec, d, o),
        "Dropdown" => dropdown(spec, d, o),
        "Relative date" => relative(o),
        _ => {
            let mut s = Scene::new(o.width, o.height);
            list(&mut s, spec, d, o, Rect::new(0.0, 0.0, o.width, o.height));
            s
        }
    }
}

fn items(d: &VisualData, o: &RenderOpts) -> Vec<usize> {
    let q = o.search.trim().to_lowercase();
    (0..d.rows.len()).filter(|r| q.is_empty() || cat_text(d, *r, 0).to_lowercase().contains(&q)).collect()
}

fn checkbox(s: &mut Scene, o: &RenderOpts, x: f32, cy: f32, on: bool, radio: bool) {
    let sz = 13.0;
    let r = Rect::new(x, cy - sz / 2.0, sz, sz);
    let border = if on { o.theme.accent } else { o.theme.foreground_muted.with_alpha(160) };
    if radio {
        s.push(Shape::Circle { cx: x + sz / 2.0, cy, r: sz / 2.0, fill: Some(o.theme.visual_background), stroke: Some(Line::solid(1.2, border)) });
        if on {
            s.push(Shape::Circle { cx: x + sz / 2.0, cy, r: sz / 4.0, fill: Some(o.theme.accent), stroke: None });
        }
    } else {
        s.push(Shape::Rect { rect: r, fill: Some(if on { o.theme.accent } else { o.theme.visual_background }), stroke: Some(Line::solid(1.2, border)), radius: 3.0 });
        if on {
            let c = o.theme.accent.contrast_text();
            s.push(Shape::Path { points: vec![(x + 3.0, cy), (x + 5.6, cy + 2.8), (x + 10.2, cy - 3.0)], closed: false, fill: None, stroke: Some(Line::solid(1.8, c)) });
        }
    }
}

fn search_box(s: &mut Scene, o: &RenderOpts, r: Rect) {
    s.push(Shape::Rect { rect: r, fill: Some(o.theme.visual_background), stroke: Some(Line::solid(1.0, o.theme.visual_border)), radius: 4.0 });
    let (ix, iy) = (r.x + 11.0, r.y + r.h / 2.0 - 1.0);
    s.push(Shape::Circle { cx: ix, cy: iy, r: 4.0, fill: None, stroke: Some(Line::solid(1.3, o.theme.foreground_muted)) });
    s.line((ix + 3.0, iy + 3.0), (ix + 6.0, iy + 6.0), Line::solid(1.3, o.theme.foreground_muted));
    let size = o.props.f32("values.size", 10.0);
    if o.search.is_empty() {
        s.text(r.x + 22.0, r.y + r.h / 2.0, "Search", size, o.theme.foreground_muted.with_alpha(170), Align::Start, Align::Center);
    } else {
        s.text(r.x + 22.0, r.y + r.h / 2.0, ell(o.search, size, r.w - 28.0), size, o.theme.foreground, Align::Start, Align::Center);
    }
    s.actions.push((r, "slicer:search".into()));
}

fn list(s: &mut Scene, spec: &VisualSpec, d: &VisualData, o: &RenderOpts, area: Rect) {
    let size = o.props.f32("values.size", 10.0) * 1.05;
    let row_h = (size * 2.2).max(22.0);
    let single = o.props.bool("slicer.single", false);
    let mut y = area.y + 2.0;
    let rows = items(d, o);
    if o.props.bool("slicer.search", true) && (d.rows.len() > 6 || !o.search.is_empty()) {
        search_box(s, o, Rect::new(area.x + 2.0, y, area.w - 4.0, 24.0));
        y += 30.0;
    }
    let fg = o.props.color_opt("values.color").unwrap_or(o.theme.foreground);
    if o.props.bool("slicer.selectAll", true) && !single && o.search.is_empty() {
        let all = !spec.selection.is_empty() && spec.selection.len() >= d.rows.len();
        checkbox(s, o, area.x + 6.0, y + row_h / 2.0, all, false);
        s.text(area.x + 26.0, y + row_h / 2.0, "Select all", size, fg, Align::Start, Align::Center);
        s.actions.push((Rect::new(area.x, y, area.w, row_h), "slicer:all".into()));
        y += row_h;
    }
    let offset = o.props.f32("scroll.offset", 0.0).max(0.0);
    let body_top = y;
    let content_h = rows.len() as f32 * row_h;
    let sel_bg = o.theme.accent.with_alpha(22);
    for (i, r) in rows.iter().enumerate() {
        let ry = body_top + i as f32 * row_h - offset;
        if ry + row_h < body_top - 0.5 {
            continue;
        }
        if ry + row_h > area.bottom() + 0.5 {
            break;
        }
        let v = d.cell(*r, 0).clone();
        let on = spec.selection.iter().any(|x| x.key() == v.key());
        if on {
            s.push(Shape::Rect { rect: Rect::new(area.x + 1.0, ry + 1.0, area.w - 2.0, row_h - 2.0), fill: Some(sel_bg), stroke: None, radius: 4.0 });
        }
        checkbox(s, o, area.x + 6.0, ry + row_h / 2.0, on, single);
        s.text(area.x + 26.0, ry + row_h / 2.0, ell(&cat_text(d, *r, 0), size, area.w - 32.0), size, fg, Align::Start, Align::Center);
        s.actions.push((Rect::new(area.x, ry, area.w, row_h), action(&v)));
    }
    if rows.is_empty() {
        s.text(area.x + area.w / 2.0, y + 14.0, "No matches", size, o.theme.foreground_muted, Align::Center, Align::Center);
    }
    let visible = area.bottom() - body_top;
    if content_h > visible + 1.0 {
        // Scrollbar + scroll region.
        let frac = (visible / content_h).clamp(0.08, 1.0);
        let max_off = content_h - visible;
        let t = (offset / max_off.max(1.0)).clamp(0.0, 1.0);
        let th = visible * frac;
        s.push(Shape::Rect { rect: Rect::new(area.right() - 4.0, body_top + (visible - th) * t, 3.0, th), fill: Some(o.theme.foreground_muted.with_alpha(90)), stroke: None, radius: 1.5 });
        s.actions.insert(0, (Rect::new(area.x, body_top, area.w, visible), format!("scroll:{content_h}")));
    }
}

fn tiles(spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let mut s = Scene::new(o.width, o.height);
    let size = o.props.f32("values.size", 10.0) * 1.05;
    let h = (size * 2.4).max(24.0);
    let (mut x, mut y) = (2.0f32, 2.0f32);
    let rows = items(d, o);
    for r in rows {
        let v = d.cell(r, 0).clone();
        let t = cat_text(d, r, 0);
        let w = (text_width(&t, size) + 24.0).min(o.width - 4.0);
        if x + w > o.width - 2.0 && x > 2.0 {
            x = 2.0;
            y += h + 6.0;
        }
        if y + h > o.height {
            break;
        }
        let on = spec.selection.iter().any(|z| z.key() == v.key());
        let rect = Rect::new(x, y, w, h);
        let (fill, tc, stroke) = if on { (o.theme.accent, o.theme.accent.contrast_text(), None) } else { (o.theme.visual_background, o.theme.foreground, Some(Line::solid(1.0, o.theme.visual_border.shade(-0.08)))) };
        s.push(Shape::Rect { rect, fill: Some(fill), stroke, radius: h / 2.0 });
        s.text(x + w / 2.0, y + h / 2.0, ell(&t, size, w - 14.0), size, tc, Align::Center, Align::Center);
        s.actions.push((rect, action(&v)));
        x += w + 6.0;
    }
    s
}

fn dropdown(spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let mut s = Scene::new(o.width, o.height);
    let size = o.props.f32("values.size", 10.0) * 1.05;
    let hh = 28.0f32.min(o.height);
    let head = Rect::new(1.0, 1.0, o.width - 2.0, hh - 2.0);
    let text = match spec.selection.len() {
        0 => "All".to_string(),
        1 => spec.selection.first().map(|v| v.text()).unwrap_or_default(),
        n => format!("Multiple selections ({n})"),
    };
    s.push(Shape::Rect { rect: head, fill: Some(o.theme.visual_background), stroke: Some(Line::solid(1.0, o.theme.visual_border.shade(-0.1))), radius: 4.0 });
    s.text(10.0, hh / 2.0, ell(&text, size, o.width - 36.0), size, o.theme.foreground, Align::Start, Align::Center);
    let (ax, ay) = (o.width - 16.0, hh / 2.0);
    s.push(Shape::Path { points: vec![(ax - 4.0, ay - 2.0), (ax, ay + 2.5), (ax + 4.0, ay - 2.0)], closed: false, fill: None, stroke: Some(Line::solid(1.4, o.theme.foreground_muted)) });
    s.actions.push((head, "slicer:dropdown".into()));
    if o.props.bool("slicer.open", false) && o.height > hh + 30.0 {
        let panel = Rect::new(1.0, hh + 2.0, o.width - 2.0, o.height - hh - 3.0);
        s.push(Shape::Rect { rect: panel, fill: Some(o.theme.visual_background), stroke: Some(Line::solid(1.0, o.theme.visual_border.shade(-0.1))), radius: 4.0 });
        list(&mut s, spec, d, o, panel.shrink(2.0));
    }
    s
}

fn range(spec: &VisualSpec, d: &VisualData, o: &RenderOpts, st: &str) -> Scene {
    let mut s = Scene::new(o.width, o.height);
    let nums: Vec<f64> = (0..d.rows.len()).filter_map(|r| d.num(r, 0)).filter(|v| v.is_finite()).collect();
    if nums.is_empty() {
        return list_fallback(spec, d, o);
    }
    let lo = nums.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = nums.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let hi = if hi <= lo { lo + 1.0 } else { hi };
    let (a, b) = spec.range.unwrap_or((lo, hi));
    let (a, b) = (a.clamp(lo, hi), b.clamp(lo, hi));
    let (a, b) = match st {
        "Less than or equal to" => (lo, b),
        "Greater than or equal to" => (a, hi),
        _ => (a.min(b), a.max(b)),
    };
    let fmt = |v: f64| {
        let c = d.cols.first();
        match c {
            Some(c) if c.col_type.is_temporal() => crate::vquery::format_value(v, &c.format, c.col_type),
            Some(c) => ValueFmt::new(&c.format).full(v),
            None => format!("{v}"),
        }
    };
    let size = o.props.f32("values.size", 10.0) * 1.05;
    let bh = 26.0;
    let two = st == "Between";
    let bw = if two { (o.width - 14.0) / 2.0 } else { o.width - 4.0 };
    let boxes: Vec<(f32, f64)> = match st {
        "Less than or equal to" => vec![(2.0, b)],
        "Greater than or equal to" => vec![(2.0, a)],
        _ => vec![(2.0, a), (2.0 + bw + 10.0, b)],
    };
    for (x, v) in &boxes {
        s.push(Shape::Rect { rect: Rect::new(*x, 2.0, bw, bh), fill: Some(o.theme.visual_background), stroke: Some(Line::solid(1.0, o.theme.visual_border.shade(-0.1))), radius: 4.0 });
        s.text(x + 8.0, 2.0 + bh / 2.0, ell(&fmt(*v), size, bw - 12.0), size, o.theme.foreground, Align::Start, Align::Center);
    }
    let ty = (bh + 26.0).min(o.height - 10.0);
    let (tx0, tx1) = (10.0, o.width - 10.0);
    let px = |v: f64| tx0 + (tx1 - tx0) * ((v - lo) / (hi - lo)) as f32;
    let track = if o.theme.visual_background.luminance() < 0.4 { o.theme.visual_border } else { Color::rgb(0xE2, 0xE6, 0xEC) };
    s.push(Shape::Rect { rect: Rect::new(tx0, ty - 2.0, tx1 - tx0, 4.0), fill: Some(track), stroke: None, radius: 2.0 });
    s.push(Shape::Rect { rect: Rect::new(px(a), ty - 2.0, (px(b) - px(a)).max(0.0), 4.0), fill: Some(o.theme.accent), stroke: None, radius: 2.0 });
    let handles: Vec<f64> = match st {
        "Less than or equal to" => vec![b],
        "Greater than or equal to" => vec![a],
        _ => vec![a, b],
    };
    for v in handles {
        s.push(Shape::Circle { cx: px(v), cy: ty, r: 7.0, fill: Some(o.theme.visual_background), stroke: Some(Line::solid(2.0, o.theme.accent)) });
    }
    s.actions.push((Rect::new(tx0 - 8.0, ty - 12.0, tx1 - tx0 + 16.0, 24.0), format!("range:{lo}:{hi}")));
    s
}

fn list_fallback(spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let mut s = Scene::new(o.width, o.height);
    list(&mut s, spec, d, o, Rect::new(0.0, 0.0, o.width, o.height));
    s
}

fn relative(o: &RenderOpts) -> Scene {
    let mut s = Scene::new(o.width, o.height);
    let size = o.props.f32("values.size", 10.0) * 1.05;
    let w = (o.width - 16.0) / 3.0;
    for (i, t) in ["Last", "1", "Months"].iter().enumerate() {
        let r = Rect::new(2.0 + i as f32 * (w + 6.0), 2.0, w, 26.0);
        s.push(Shape::Rect { rect: r, fill: Some(o.theme.visual_background), stroke: Some(Line::solid(1.0, o.theme.visual_border.shade(-0.1))), radius: 4.0 });
        s.text(r.x + 8.0, r.y + 13.0, *t, size, o.theme.foreground, Align::Start, Align::Center);
    }
    s
}
