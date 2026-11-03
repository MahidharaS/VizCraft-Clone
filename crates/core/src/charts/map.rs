//! Map and filled map: bubbles at latitude/longitude (or looked-up location names) on an
//! equirectangular projection with a light graticule and a coarse world outline; zooms to the
//! points' bounding box for regional data. The filled map colours each location by value
//! (gradient) with a colour-scale legend.

use super::RenderOpts;
use super::axis::*;
use super::common::*;
use super::world;
use crate::color::Color;
use crate::doc::*;
use crate::scene::{Align, Line, Rect, Scene, Shape, text_width};
use crate::vquery::VisualData;

struct Pt {
    row: usize,
    lat: f32,
    lon: f32,
}

/// Clips a polygon to a rectangle (Sutherland–Hodgman).
pub fn clip_polygon(pts: &[(f32, f32)], r: Rect) -> Vec<(f32, f32)> {
    let edges: [(u8, f32); 4] = [(0, r.x), (1, r.right()), (2, r.y), (3, r.bottom())];
    let mut out = pts.to_vec();
    for (kind, v) in edges {
        let inside = |p: (f32, f32)| match kind {
            0 => p.0 >= v,
            1 => p.0 <= v,
            2 => p.1 >= v,
            _ => p.1 <= v,
        };
        let cross = |a: (f32, f32), b: (f32, f32)| -> (f32, f32) {
            if kind < 2 {
                let t = (v - a.0) / (b.0 - a.0 + f32::EPSILON);
                (v, a.1 + t * (b.1 - a.1))
            } else {
                let t = (v - a.1) / (b.1 - a.1 + f32::EPSILON);
                (a.0 + t * (b.0 - a.0), v)
            }
        };
        let input = std::mem::take(&mut out);
        let n = input.len();
        for i in 0..n {
            let (Some(cur), Some(prev)) = (input.get(i).copied(), input.get((i + n - 1) % n).copied()) else { continue };
            match (inside(cur), inside(prev)) {
                (true, true) => out.push(cur),
                (true, false) => {
                    out.push(cross(prev, cur));
                    out.push(cur);
                }
                (false, true) => out.push(cross(prev, cur)),
                _ => {}
            }
        }
        if out.is_empty() {
            break;
        }
    }
    out
}

pub fn render(spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let lat_c = d.first("lat").filter(|c| d.cols.get(*c).is_some_and(|x| x.is_value));
    let lon_c = d.first("lon").filter(|c| d.cols.get(*c).is_some_and(|x| x.is_value));
    let loc = d.first("location").filter(|c| d.cols.get(*c).is_some_and(|x| !x.is_value));
    if lat_c.is_none() && loc.is_none() {
        return empty(o, "Add a field to Location, or Latitude and Longitude");
    }
    let size_c = d.first("size").or_else(|| d.first("values")).filter(|c| d.cols.get(*c).is_some_and(|x| x.is_value));
    let legend_c = d.first("legend").filter(|c| d.cols.get(*c).is_some_and(|x| !x.is_value));
    let mut pts = Vec::new();
    let mut missing = 0;
    for r in 0..d.rows.len() {
        let ll = match (lat_c, lon_c) {
            (Some(a), Some(b)) => d.num(r, a).zip(d.num(r, b)).map(|(a, b)| (a as f32, b as f32)).filter(|(a, b)| a.is_finite() && b.is_finite() && a.abs() <= 90.0 && b.abs() <= 180.0),
            _ => None,
        };
        let ll = ll.or_else(|| loc.and_then(|c| world::lookup(&d.cell(r, c).text())));
        match ll {
            Some((lat, lon)) => pts.push(Pt { row: r, lat, lon }),
            None => missing += 1,
        }
    }
    let filled = spec.kind == VisualKind::FilledMap;
    let mut s = Scene::new(o.width, o.height);
    let mut area = Rect::new(0.0, 0.0, o.width, o.height);
    // Legend (categories) or a gradient bar (filled map).
    let lvals = legend_c.map(|c| d.distinct(c)).unwrap_or_default();
    if let Some(lc) = legend_c {
        let items: Vec<LegendItem> = lvals.iter().enumerate().map(|(i, v)| LegendItem { label: v.text(), color: o.theme.series_color(i), line: false, hit: None, dimmed: false }).collect();
        area = legend(&mut s, o, &items, &d.cols.get(lc).map(|c| c.label.clone()).unwrap_or_default(), Rect::new(4.0, 4.0, o.width - 8.0, o.height - 8.0));
        area = Rect::new(0.0, area.y - 4.0, o.width, o.height - area.y + 4.0);
    }
    // View: world, or the points' bounding box when they are regional.
    let (mut lon0, mut lon1, mut lat0, mut lat1) = (-170.0f32, 190.0f32, -56.0f32, 78.0f32);
    if !pts.is_empty() {
        let (a, b) = (pts.iter().map(|p| p.lon).fold(f32::INFINITY, f32::min), pts.iter().map(|p| p.lon).fold(f32::NEG_INFINITY, f32::max));
        let (c, e) = (pts.iter().map(|p| p.lat).fold(f32::INFINITY, f32::min), pts.iter().map(|p| p.lat).fold(f32::NEG_INFINITY, f32::max));
        if b - a < 90.0 && e - c < 60.0 {
            // At least ~22° x 16° of view (one country / one point still shows its region).
            let pad_x = ((b - a) * 0.3).max((22.0 - (b - a)) / 2.0).max(3.0);
            let pad_y = ((e - c) * 0.3).max((16.0 - (e - c)) / 2.0).max(2.5);
            lon0 = a - pad_x;
            lon1 = b + pad_x;
            lat0 = c - pad_y;
            lat1 = e + pad_y;
        } else {
            lon0 = -180.0;
            lon1 = 180.0;
        }
    }
    // Aspect: equirectangular with cos(mid-latitude) correction; fit the view into the area.
    let k = ((lat0 + lat1) / 2.0).to_radians().cos().max(0.2);
    let (vw, vh) = ((lon1 - lon0) * k, lat1 - lat0);
    let scale = (area.w / vw).min(area.h / vh);
    // Expand the view to fill the area.
    let extra_x = (area.w / scale - vw) / k / 2.0;
    let extra_y = (area.h / scale - vh) / 2.0;
    let (lon0, lon1, lat0, lat1) = (lon0 - extra_x, lon1 + extra_x, lat0 - extra_y, lat1 + extra_y);
    let proj = |lon: f32, lat: f32| -> (f32, f32) { (area.x + (lon - lon0) * k * scale, area.y + (lat1 - lat) * scale) };
    // Format-pane colours; their light defaults give way to theme colours on dark themes.
    let dark = o.theme.visual_background.luminance() < 0.4;
    let pick = |key: &str, light_default: Color, dark_c: Color, light_c: Color| match o.props.color_opt(key) {
        Some(c) if !(dark && c == light_default) => c,
        _ => if dark { dark_c } else { light_c },
    };
    let water = pick("map.water", Color::rgb(0xF7, 0xFA, 0xFC), o.theme.visual_background.shade(0.03), Color::rgb(0xF4, 0xF8, 0xFB));
    let land = pick("map.land", Color::rgb(0xE9, 0xEE, 0xF3), o.theme.visual_border, Color::rgb(0xE4, 0xE9, 0xEF));
    s.push(Shape::Rect { rect: area, fill: Some(water), stroke: None, radius: 4.0 });
    // Graticule.
    if o.props.bool("map.graticule", true) {
        let span = lon1 - lon0;
        let step = if span > 150.0 { 30.0 } else if span > 60.0 { 15.0 } else if span > 25.0 { 10.0 } else { 5.0 };
        let gl = Line::solid(0.7, land.shade(-0.06).with_alpha(110));
        let mut x = (lon0 / step).ceil() * step;
        while x <= lon1 {
            let (a, b) = (proj(x, lat1.min(85.0)), proj(x, lat0.max(-85.0)));
            s.line((a.0, a.1.max(area.y)), (b.0, b.1.min(area.bottom())), gl.clone());
            x += step;
        }
        let mut y = (lat0 / step).ceil() * step;
        while y <= lat1 {
            let (a, b) = (proj(lon0, y), proj(lon1, y));
            s.line(a, b, gl.clone());
            y += step;
        }
    }
    // Land (with copies shifted by ±360° for views crossing the date line).
    let coast = Line::solid(0.8, land.shade(-0.12));
    for shift in [-360.0f32, 0.0, 360.0] {
        for poly in world::LAND {
            let (pa, pb) = poly.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |acc, p| (acc.0.min(p.0 + shift), acc.1.max(p.0 + shift)));
            if pb < lon0 || pa > lon1 {
                continue;
            }
            let pp: Vec<(f32, f32)> = poly.iter().map(|(x, y)| proj(*x + shift, *y)).collect();
            let c = clip_polygon(&pp, area);
            if c.len() >= 3 {
                s.push(Shape::Path { points: c, closed: true, fill: Some(land), stroke: Some(coast.clone()) });
            }
        }
    }
    if lon1 - lon0 < 80.0 {
        for b in world::BORDERS {
            let pp: Vec<(f32, f32)> = b.iter().map(|(x, y)| proj(*x, *y)).filter(|p| area.contains(p.0, p.1)).collect();
            if pp.len() >= 2 {
                s.push(Shape::Path { points: pp, closed: false, fill: None, stroke: Some(Line::from_style(1.0, land.shade(-0.3), "dash")) });
            }
        }
    }
    if pts.is_empty() {
        s.text(area.x + area.w / 2.0, area.y + area.h / 2.0, "No locations found", 11.0, o.theme.foreground_muted, Align::Center, Align::Center);
        return s;
    }
    // Bubbles.
    let vals: Vec<f64> = pts.iter().map(|p| size_c.and_then(|c| d.num(p.row, c)).unwrap_or(0.0)).collect();
    let vmax = vals.iter().cloned().fold(0.0f64, |a, b| a.max(b.abs()));
    let vmin = vals.iter().cloned().fold(f64::INFINITY, f64::min);
    let maxr = o.props.f32("bubbles.size", 10.0) * if filled { 1.6 } else { 2.2 };
    let base = data_alpha(o, series_color(o, 0, ""));
    let mut order: Vec<usize> = (0..pts.len()).collect();
    order.sort_by(|a, b| vals.get(*b).partial_cmp(&vals.get(*a)).unwrap_or(std::cmp::Ordering::Equal));
    let mut placed: Vec<Rect> = Vec::new();
    let mut labels = Vec::new();
    for i in order {
        let (Some(p), Some(v)) = (pts.get(i), vals.get(i)) else { continue };
        let (x, y) = proj(p.lon, p.lat);
        if !area.contains(x, y) {
            continue;
        }
        let r = if filled { maxr } else if vmax > 0.0 && size_c.is_some() { (maxr * (v.abs() / vmax).sqrt() as f32).max(3.0) } else { 6.0 };
        let mut color = match legend_c {
            Some(lc) => o.theme.series_color(lvals.iter().position(|x| x.key() == d.cell(p.row, lc).key()).unwrap_or(0)),
            None if filled => seq_color(o, if vmax > vmin { (v - vmin) / (vmax - vmin) } else { 1.0 } * 0.85 + 0.15),
            None => base,
        };
        if let Some(c) = o.props.point_colors.get(&color_key(d, p.row)) {
            color = *c;
        }
        let h = hit(d, p.row, POINT_ROLES);
        let dimmed = dim(o, &h.point);
        let shape = if filled {
            // A soft hexagon "region" tile.
            let pts6: Vec<(f32, f32)> = (0..6).map(|k| {
                let a = (k as f32 * 60.0 + 30.0).to_radians();
                (x + r * a.cos(), y + r * a.sin())
            }).collect();
            Shape::Path { points: pts6, closed: true, fill: Some(color.with_alpha(230)), stroke: Some(Line::solid(1.5, o.theme.visual_background)) }
        } else {
            Shape::Circle { cx: x, cy: y, r, fill: Some(color.with_alpha(190)), stroke: Some(Line::solid(1.2, o.theme.visual_background)) }
        };
        s.push_hit(shape, h, dimmed);
        if let Some(lc) = loc {
            labels.push((x, y, r, cat_text(d, p.row, lc)));
        }
    }
    if pts.len() <= 25 {
        for (x, y, r, t) in labels {
            let w = text_width(&t, 9.0);
            let rect = Rect::new(x + r + 3.0, y - 6.0, w, 12.0);
            if rect.right() < area.right() && !placed.iter().any(|q| super::scatter::overlaps(q, &rect)) {
                s.push(Shape::Text { x: rect.x, y, text: t, size: 9.0, color: o.theme.foreground.with_alpha(200), bold: false, italic: false, h: Align::Start, v: Align::Center, angle: 0.0 });
                placed.push(rect);
            }
        }
    }
    // Gradient legend for the filled map.
    if filled && size_c.is_some() && vmax > vmin && area.w > 160.0 {
        let fmt = size_c.map(|c| ValueFmt::of_col(d, c)).unwrap_or_default();
        let (w, h) = (110.0, 8.0);
        let (x, y) = (area.right() - w - 10.0, area.bottom() - 26.0);
        s.push(Shape::Rect { rect: Rect::new(x - 6.0, y - 6.0, w + 12.0, 30.0), fill: Some(o.theme.visual_background.with_alpha(220)), stroke: None, radius: 4.0 });
        for k in 0..20 {
            let t = k as f64 / 19.0;
            s.rect(Rect::new(x + w * k as f32 / 20.0, y, w / 20.0 + 0.5, h), seq_color(o, t * 0.85 + 0.15));
        }
        s.text(x, y + h + 2.0, fmt.short(vmin), 8.0, o.theme.foreground_muted, Align::Start, Align::Start);
        s.text(x + w, y + h + 2.0, fmt.short(vmax), 8.0, o.theme.foreground_muted, Align::End, Align::Start);
    }
    if missing > 0 {
        s.text(area.x + 6.0, area.bottom() - 6.0, format!("{missing} location(s) not found"), 8.5, o.theme.foreground_muted, Align::Start, Align::End);
    }
    s
}
