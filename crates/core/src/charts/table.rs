//! Tables, matrices (stepped row hierarchy with +/- expanders, column groups, subtotals and
//! totals), heat maps and highlight tables. Conditional formatting comes from
//! `Props::cell_colors` (background / font / data bars / icons).

use std::collections::HashMap;

use super::RenderOpts;
use super::common::*;
use crate::color::Color;
use crate::doc::*;
use crate::scalar::Scalar;
use crate::scene::{Align, Hit, Line, Rect, Scene, Shape, text_width};
use crate::vquery::VisualData;

/// Resolved table styling.
struct Style {
    hsize: f32,
    vsize: f32,
    row_h: f32,
    head_bg: Color,
    head_fg: Color,
    head_bold: bool,
    fg: Color,
    bg: Option<Color>,
    alt: Option<Color>,
    grid: Color,
    total_bg: Color,
    total_bold: bool,
    head_line: Color,
}

fn style(o: &RenderOpts) -> Style {
    let preset = o.props.text("table.style", "Default");
    let hsize = o.props.f32("headers.size", 10.0);
    let vsize = o.props.f32("values.size", 10.0);
    let pad = o.props.f32("table.rowPadding", 4.0).clamp(0.0, 30.0) * if preset == "Condensed" { 0.4 } else { 1.0 };
    let dark = o.theme.visual_background.luminance() < 0.4;
    let band = if dark { o.theme.visual_background.shade(0.05) } else { Color::rgb(0xF6, 0xF7, 0xF9) };
    let (head_bg, head_fg) = match preset.as_str() {
        "Bold header" | "Contrast" => (o.theme.accent, o.theme.accent.contrast_text()),
        "Flashy rows" => (o.theme.accent.shade(0.75), o.theme.foreground),
        _ => (o.theme.visual_background, o.theme.foreground),
    };
    let alt = match preset.as_str() {
        "Minimal" | "None" => None,
        "Flashy rows" => Some(o.theme.accent.shade(0.88)),
        "Alternating rows" | "Default" | "Bold header" | "Condensed" | "Contrast" => o.props.bool("values.banded", true).then_some(band),
        _ => None,
    };
    Style {
        hsize,
        vsize,
        row_h: vsize * 1.3 + pad * 2.0 + 2.0,
        head_bg: o.props.color_opt("headers.background").unwrap_or(head_bg),
        head_fg: o.props.color_opt("headers.color").unwrap_or(head_fg),
        head_bold: o.props.bool("headers.bold", true),
        fg: o.props.color_opt("values.color").unwrap_or(o.theme.foreground),
        bg: o.props.color_opt("values.background"),
        alt: o.props.color_opt("values.altBackground").or(alt),
        grid: grid_color(o),
        total_bg: o.props.color_opt("totals.background").unwrap_or(if dark { o.theme.visual_background.shade(0.08) } else { Color::rgb(0xF1, 0xF3, 0xF6) }),
        total_bold: o.props.bool("totals.bold", true),
        head_line: if dark { o.theme.foreground_muted } else { Color::rgb(0xC9, 0xCE, 0xD6) },
    }
}

fn text_shape(x: f32, y: f32, t: String, size: f32, color: Color, bold: bool, h: Align) -> Shape {
    Shape::Text { x, y, text: t, size, color, bold, italic: false, h, v: Align::Center, angle: 0.0 }
}

/// Fits column widths: natural widths, then grown or shrunk to the available width.
fn fit_widths(natural: &[f32], avail: f32, min: f32) -> Vec<f32> {
    let total: f32 = natural.iter().sum();
    if total <= 0.0 {
        return natural.to_vec();
    }
    if total < avail {
        let extra = (avail - total) / natural.len().max(1) as f32;
        return natural.iter().map(|w| w + extra).collect();
    }
    let k = avail / total;
    natural.iter().map(|w| (w * k).max(min)).collect()
}

/// Order of a table's columns: the user's order in the Columns well.
fn column_order(spec: &VisualSpec, d: &VisualData) -> Vec<usize> {
    let mut out = Vec::new();
    for fu in spec.well("columns") {
        let label = fu.label();
        if let Some(i) = d.cols.iter().enumerate().position(|(i, c)| !out.contains(&i) && (c.label == label || c.field == fu.field)) {
            out.push(i);
        }
    }
    for i in 0..d.cols.len() {
        if !out.contains(&i) {
            out.push(i);
        }
    }
    out
}

pub fn table(spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    if d.cols.is_empty() {
        return empty(o, "Add fields to Columns");
    }
    let st = style(o);
    let mut s = Scene::new(o.width, o.height);
    let order = column_order(spec, d);
    let sample = d.rows.len().min(300);
    let natural: Vec<f32> = order
        .iter()
        .map(|c| {
            let hw = text_width(&d.cols.get(*c).map(|x| x.label.clone()).unwrap_or_default(), st.hsize) * if st.head_bold { 1.08 } else { 1.0 };
            let vw = (0..sample).map(|r| text_width(&d.text(r, *c), st.vsize)).fold(0.0, f32::max);
            let tw = d.totals.get(*c).filter(|v| !v.is_null()).map(|_| text_width(&d.text_total(*c), st.vsize) * 1.08).unwrap_or(0.0);
            hw.max(vw).max(tw).min(320.0) + 18.0
        })
        .collect();
    let widths = fit_widths(&natural, o.width, 36.0);
    let xs: Vec<f32> = widths.iter().scan(0.0, |acc, w| {
        let x = *acc;
        *acc += w;
        Some(x)
    }).collect();
    let head_h = st.hsize * 1.3 + 14.0;
    let show_totals = o.props.bool("totals.show", true) && d.totals.iter().any(|v| !v.is_null()) && !d.rows.is_empty();
    let foot_h = if show_totals { st.row_h + 2.0 } else { 0.0 };
    let body = Rect::new(0.0, head_h, o.width, (o.height - head_h - foot_h).max(0.0));
    let offset = o.props.f32("scroll.offset", 0.0).max(0.0);
    let content_h = d.rows.len() as f32 * st.row_h;
    // Column maxima for data bars.
    let col_max: HashMap<usize, f64> = order.iter().map(|c| (*c, (0..d.rows.len()).filter_map(|r| d.num(r, *c)).fold(0.0f64, |a, b| a.max(b.abs())))).collect();
    if let Some(bg) = st.bg {
        s.rect(body, bg);
    }
    let first = ((offset / st.row_h).floor() as usize).min(d.rows.len());
    let last = (((offset + body.h) / st.row_h).ceil() as usize + 1).min(d.rows.len());
    let align_right = |c: usize| d.cols.get(c).is_some_and(|x| x.is_value || x.col_type.is_numeric());
    for r in first..last {
        let y = body.y + r as f32 * st.row_h - offset;
        if y + st.row_h <= body.y || y >= body.bottom() {
            continue;
        }
        let rect = Rect::new(0.0, y.max(body.y), o.width, (st.row_h - (body.y - y).max(0.0)).min(body.bottom() - y.max(body.y)));
        if r % 2 == 1
            && let Some(a) = st.alt
        {
            s.rect(rect, a);
        }
        let rk = crate::fx::point_key(&d.identity(r, &[]));
        let full = y >= body.y - 0.5 && y + st.row_h <= body.bottom() + 0.5;
        for (k, c) in order.iter().enumerate() {
            let (Some(x), Some(w)) = (xs.get(k), widths.get(k)) else { continue };
            let label = d.cols.get(*c).map(|x| x.label.clone()).unwrap_or_default();
            let cell = Rect::new(*x, rect.y, *w, rect.h);
            if let Some(bg) = o.props.cell_colors.get(&(rk.clone(), label.clone(), "background".into())) {
                s.rect(cell, *bg);
            }
            if let Some(bc) = o.props.cell_colors.get(&(rk.clone(), label.clone(), "dataBar".into()))
                && let (Some(v), Some(m)) = (d.num(r, *c), col_max.get(c))
                && *m > 0.0
            {
                let bw = (*w - 8.0) * (v.abs() / m) as f32;
                s.push(Shape::Rect { rect: Rect::new(x + 4.0, cell.y + 3.0, bw.max(0.0), (cell.h - 6.0).max(0.0)), fill: Some(bc.with_alpha(110)), stroke: None, radius: 2.0 });
            }
            if !full {
                continue;
            }
            let cell_bg = o.props.cell_colors.get(&(rk.clone(), label.clone(), "background".into())).copied();
            let fg = o.props.cell_colors.get(&(rk.clone(), label.clone(), "font".into())).copied().or(cell_bg.map(|b| b.contrast_text())).unwrap_or(st.fg);
            let mut tx = x + 8.0;
            if let Some(ic) = o.props.cell_colors.get(&(rk.clone(), label.clone(), "icon".into())) {
                s.push(Shape::Circle { cx: x + 10.0, cy: y + st.row_h / 2.0, r: 4.0, fill: Some(*ic), stroke: None });
                tx += 10.0;
            }
            let t = ell(&d.text(r, *c), st.vsize, w - 14.0 - (tx - x - 8.0));
            if align_right(*c) {
                s.push(text_shape(x + w - 8.0, y + st.row_h / 2.0, t, st.vsize, fg, false, Align::End));
            } else {
                s.push(text_shape(tx, y + st.row_h / 2.0, t, st.vsize, fg, false, Align::Start));
            }
        }
        s.line((0.0, rect.bottom()), (o.width, rect.bottom()), Line::solid(0.6, st.grid.with_alpha(140)));
        let point = d.identity(r, &[]);
        let dimmed = dim(o, &point);
        s.push_hit(Shape::Rect { rect, fill: None, stroke: None, radius: 0.0 }, Hit { point, tooltip: tooltip(d, r) }, dimmed);
    }
    // Header (sticky).
    s.rect(Rect::new(0.0, 0.0, o.width, head_h), st.head_bg);
    let halign = o.props.text("headers.align", "Left");
    for (k, c) in order.iter().enumerate() {
        let (Some(x), Some(w)) = (xs.get(k), widths.get(k)) else { continue };
        let t = ell(&d.cols.get(*c).map(|x| x.label.clone()).unwrap_or_default(), st.hsize * 1.06, w - 14.0);
        let (tx, h) = if align_right(*c) && halign != "Center" || halign == "Right" { (x + w - 8.0, Align::End) } else if halign == "Center" { (x + w / 2.0, Align::Center) } else { (x + 8.0, Align::Start) };
        s.push(text_shape(tx, head_h / 2.0, t, st.hsize, st.head_fg, st.head_bold, h));
    }
    s.line((0.0, head_h), (o.width, head_h), Line::solid(1.2, st.head_line));
    if d.rows.is_empty() {
        s.text(o.width / 2.0, head_h + 20.0, "No data", st.vsize, o.theme.foreground_muted, Align::Center, Align::Center);
    }
    // Totals row (sticky at the bottom of the rows).
    if show_totals {
        let ty = (body.y + content_h - offset).min(body.bottom()) + 1.0;
        let tr = Rect::new(0.0, ty, o.width, st.row_h);
        s.rect(tr, st.total_bg);
        s.line((0.0, ty), (o.width, ty), Line::solid(1.2, st.head_line));
        for (k, c) in order.iter().enumerate() {
            let (Some(x), Some(w)) = (xs.get(k), widths.get(k)) else { continue };
            let t = if k == 0 && d.totals.get(*c).is_none_or(Scalar::is_null) { "Total".to_string() } else { d.text_total(*c) };
            if t.is_empty() {
                continue;
            }
            let t = ell(&t, st.vsize, w - 14.0);
            if align_right(*c) {
                s.push(text_shape(x + w - 8.0, ty + st.row_h / 2.0, t, st.vsize, st.fg, st.total_bold, Align::End));
            } else {
                s.push(text_shape(x + 8.0, ty + st.row_h / 2.0, t, st.vsize, st.fg, st.total_bold, Align::Start));
            }
        }
    }
    if content_h > body.h + 1.0 {
        scrollbar(&mut s, o, body, content_h, offset);
    }
    s
}

fn scrollbar(s: &mut Scene, o: &RenderOpts, body: Rect, content_h: f32, offset: f32) {
    let frac = (body.h / content_h).clamp(0.06, 1.0);
    let th = body.h * frac;
    let t = (offset / (content_h - body.h).max(1.0)).clamp(0.0, 1.0);
    s.push(Shape::Rect { rect: Rect::new(body.right() - 5.0, body.y + (body.h - th) * t, 4.0, th), fill: Some(o.theme.foreground_muted.with_alpha(90)), stroke: None, radius: 2.0 });
    s.actions.insert(0, (body, format!("scroll:{content_h}")));
}

impl VisualData {
    /// Display text of a total (formatted with the column's format).
    pub fn text_total(&self, col: usize) -> String {
        match (self.totals.get(col), self.cols.get(col)) {
            (Some(Scalar::Num(n)), Some(c)) => crate::vquery::format_value(*n, &c.format, c.col_type),
            (Some(Scalar::Null) | None, _) => String::new(),
            (Some(v), _) => v.text(),
        }
    }
}

fn key_of(vals: &[Scalar]) -> String {
    vals.iter().map(Scalar::key).collect::<Vec<_>>().join("\u{1}")
}

/// The JSON row key used by `expand:` / `collapse:` actions and `VisualSpec::expanded`.
pub fn row_key(path: &[Scalar]) -> String {
    serde_json::to_string(path).unwrap_or_default()
}

pub fn matrix(spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let rcols = d.role("rows");
    let ccols = d.role("columns");
    let vcols = d.values();
    if rcols.is_empty() && ccols.is_empty() && vcols.is_empty() {
        return empty(o, "Add fields to Rows, Columns or Values");
    }
    let nr = rcols.len();
    let nc = ccols.len();
    let nv = vcols.len();
    let st = style(o);
    let mut s = Scene::new(o.width, o.height);
    let fmt = |vi: usize, v: &Scalar| -> String {
        match (v, vcols.get(vi).and_then(|c| d.cols.get(*c))) {
            (Scalar::Num(n), Some(c)) => crate::vquery::format_value(*n, &c.format, c.col_type),
            (Scalar::Null, _) => String::new(),
            (v, _) => v.text(),
        }
    };
    // Column groups.
    let mut groups: Vec<Vec<Scalar>> = Vec::new();
    let mut gindex: HashMap<String, usize> = HashMap::new();
    if nc > 0 {
        let src: Vec<Vec<Scalar>> = match d.subtotals.get(&0) {
            Some(rows) if !rows.is_empty() => rows.iter().map(|r| r.iter().take(nc).cloned().collect()).collect(),
            _ => d.rows.iter().map(|r| ccols.iter().map(|c| r.get(*c).cloned().unwrap_or_default()).collect()).collect(),
        };
        for g in src {
            let k = key_of(&g);
            if let std::collections::hash_map::Entry::Vacant(e) = gindex.entry(k) {
                e.insert(groups.len());
                groups.push(g);
            }
        }
    } else {
        groups.push(vec![]);
        gindex.insert(String::new(), 0);
    }
    let show_totals = o.props.bool("totals.show", true);
    let row_totals_col = nc > 0 && show_totals && o.props.bool("totals.columnSubtotals", true);
    // Lookups: level j (1..=nr) → (row path key, group key) → values.
    let mut cells: HashMap<(usize, String, String), Vec<Scalar>> = HashMap::new();
    let mut paths: Vec<Vec<Vec<Scalar>>> = vec![Vec::new(); nr + 1];
    let mut seen: Vec<std::collections::HashSet<String>> = vec![Default::default(); nr + 1];
    let mut add_level = |j: usize, rows: &[Vec<Scalar>], with_cols: bool, cells: &mut HashMap<(usize, String, String), Vec<Scalar>>| {
        for r in rows {
            let path: Vec<Scalar> = r.iter().take(j).cloned().collect();
            let g: Vec<Scalar> = if with_cols { r.iter().skip(j).take(nc).cloned().collect() } else { vec![] };
            let vals: Vec<Scalar> = r.iter().skip(j + if with_cols { nc } else { 0 }).take(nv).cloned().collect();
            let pk = key_of(&path);
            if with_cols {
                if let (Some(se), Some(pl)) = (seen.get_mut(j), paths.get_mut(j))
                    && se.insert(pk.clone())
                {
                    pl.push(path.clone());
                }
                cells.insert((j, pk, key_of(&g)), vals);
            } else {
                cells.insert((j, pk, "\u{2}total".into()), vals);
            }
        }
    };
    // Leaf level from the data rows (rows + columns + values in that order).
    let leaf: Vec<Vec<Scalar>> = d.rows.iter().map(|r| {
        let mut v: Vec<Scalar> = rcols.iter().chain(ccols.iter()).chain(vcols.iter()).map(|c| r.get(*c).cloned().unwrap_or_default()).collect();
        v.truncate(nr + nc + nv);
        v
    }).collect();
    add_level(nr, &leaf, true, &mut cells);
    for j in 0..nr {
        if let Some(rows) = d.subtotals.get(&j) {
            add_level(j, rows, true, &mut cells);
        }
    }
    for j in 0..=nr {
        if let Some(rows) = d.subtotals.get(&(100 + j)) {
            add_level(j, rows, false, &mut cells);
        }
    }
    // Visible rows (stepped): depth-first over expanded nodes.
    let expanded = |path: &[Scalar]| -> bool { spec.drill.expand_to >= path.len() || spec.expanded.contains(&row_key(path)) };
    let mut visible: Vec<Vec<Scalar>> = Vec::new();
    fn walk(level: usize, parent: &[Scalar], nr: usize, paths: &[Vec<Vec<Scalar>>], expanded: &dyn Fn(&[Scalar]) -> bool, out: &mut Vec<Vec<Scalar>>) {
        let Some(list) = paths.get(level) else { return };
        for p in list.iter().filter(|p| p.len() == level && p.get(..parent.len()).is_some_and(|x| x == parent)) {
            out.push(p.clone());
            if level < nr && expanded(p) {
                walk(level + 1, p, nr, paths, expanded, out);
            }
        }
    }
    if nr > 0 {
        walk(1, &[], nr, &paths, &expanded, &mut visible);
    }
    // Columns: (group index or None = total, value index).
    let mut colspec: Vec<(Option<usize>, usize)> = Vec::new();
    for g in 0..groups.len() {
        for v in 0..nv.max(1) {
            colspec.push((Some(g), v));
        }
    }
    if row_totals_col {
        for v in 0..nv.max(1) {
            colspec.push((None, v));
        }
    }
    let cell_val = |path: &[Scalar], col: &(Option<usize>, usize)| -> Scalar {
        let j = path.len();
        let pk = key_of(path);
        let gk = match col.0 {
            Some(g) => groups.get(g).map(|x| key_of(x)).unwrap_or_default(),
            None => "\u{2}total".into(),
        };
        cells.get(&(j, pk, gk)).and_then(|v| v.get(col.1)).cloned().unwrap_or_default()
    };
    // Widths.
    let indent = o.props.f32("matrix.indent", 16.0);
    let icons = o.props.bool("matrix.expandIcons", true) && nr > 1;
    let row_label = rcols.iter().filter_map(|c| d.cols.get(*c)).map(|c| c.label.clone()).collect::<Vec<_>>().join(" / ");
    let label_w = visible
        .iter()
        .map(|p| (p.len().saturating_sub(1)) as f32 * indent + if icons { 16.0 } else { 0.0 } + p.last().map(|v| text_width(&v.text(), st.vsize)).unwrap_or(0.0))
        .fold(text_width(&row_label, st.hsize) * 1.08, f32::max)
        .min(o.width * 0.45)
        + 18.0;
    let natural: Vec<f32> = colspec
        .iter()
        .map(|c| {
            let vname = vcols.get(c.1).and_then(|x| d.cols.get(*x)).map(|x| x.label.clone()).unwrap_or_default();
            let gname = c.0.and_then(|g| groups.get(g)).map(|g| g.iter().map(Scalar::text).collect::<Vec<_>>().join(" ")).unwrap_or_else(|| "Total".into());
            let hw = text_width(&vname, st.hsize) * 1.08;
            let gw = if nv > 1 { 0.0 } else { text_width(&gname, st.hsize) * 1.08 };
            let vw = visible.iter().take(200).map(|p| text_width(&fmt(c.1, &cell_val(p, c)), st.vsize)).fold(0.0, f32::max);
            hw.max(gw).max(vw).max(36.0) + 18.0
        })
        .collect();
    let avail = (o.width - label_w).max(40.0);
    let widths = fit_widths(&natural, avail, 34.0);
    let header_rows = if nc > 0 { if nv > 1 { 2 } else { 1 } } else { 1 };
    let hh = st.hsize * 1.3 + 12.0;
    let head_h = hh * header_rows as f32;
    let show_grand = show_totals && o.props.bool("totals.rowSubtotals", true) && nr > 0;
    let foot_h = if show_grand { st.row_h + 2.0 } else { 0.0 };
    let body = Rect::new(0.0, head_h, o.width, (o.height - head_h - foot_h).max(0.0));
    let offset = o.props.f32("scroll.offset", 0.0).max(0.0);
    let content_h = visible.len() as f32 * st.row_h;
    let xs: Vec<f32> = widths.iter().scan(label_w, |acc, w| {
        let x = *acc;
        *acc += w;
        Some(x)
    }).collect();
    // Body rows.
    let row_ids: Vec<usize> = (0..visible.len()).collect();
    for i in row_ids {
        let Some(path) = visible.get(i) else { continue };
        let y = body.y + i as f32 * st.row_h - offset;
        if y < body.y - 0.5 || y + st.row_h > body.bottom() + 0.5 {
            continue;
        }
        let depth = path.len();
        let is_parent = depth < nr;
        let rect = Rect::new(0.0, y, o.width, st.row_h);
        if i % 2 == 1
            && let Some(a) = st.alt
        {
            s.rect(rect, a);
        }
        if is_parent && expanded(path) {
            s.rect(rect, st.total_bg.with_alpha(150));
        }
        let x0 = 8.0 + (depth.saturating_sub(1)) as f32 * indent;
        let mut tx = x0;
        if icons && is_parent {
            let open = expanded(path);
            let b = Rect::new(x0, y + st.row_h / 2.0 - 5.5, 11.0, 11.0);
            s.push(Shape::Rect { rect: b, fill: Some(o.theme.visual_background), stroke: Some(Line::solid(1.0, o.theme.foreground_muted.with_alpha(170))), radius: 2.0 });
            s.line((b.x + 2.5, b.y + 5.5), (b.right() - 2.5, b.y + 5.5), Line::solid(1.2, o.theme.foreground));
            if !open {
                s.line((b.x + 5.5, b.y + 2.5), (b.x + 5.5, b.bottom() - 2.5), Line::solid(1.2, o.theme.foreground));
            }
            s.actions.push((Rect::new(b.x - 3.0, b.y - 3.0, b.w + 6.0, b.h + 6.0), format!("{}:{}", if open { "collapse" } else { "expand" }, row_key(path))));
            tx += 16.0;
        } else if icons {
            tx += 16.0;
        }
        let label = path.last().map(|v| if v.is_null() { "(Blank)".to_string() } else { v.text() }).unwrap_or_default();
        let bold = is_parent && st.total_bold;
        s.push(text_shape(tx, y + st.row_h / 2.0, ell(&label, st.vsize, label_w - tx - 4.0), st.vsize, st.fg, bold, Align::Start));
        let rk = key_of(path);
        for (k, c) in colspec.iter().enumerate() {
            let (Some(x), Some(w)) = (xs.get(k), widths.get(k)) else { continue };
            let v = cell_val(path, c);
            let vlabel = vcols.get(c.1).and_then(|x| d.cols.get(*x)).map(|x| x.label.clone()).unwrap_or_default();
            let cell_bg = o.props.cell_colors.get(&(rk.clone(), vlabel.clone(), "background".into())).copied();
            if let Some(bg) = cell_bg {
                s.rect(Rect::new(*x, y, *w, st.row_h), bg);
            }
            let fg = o.props.cell_colors.get(&(rk.clone(), vlabel, "font".into())).copied().or(cell_bg.map(|b| b.contrast_text())).unwrap_or(st.fg);
            s.push(text_shape(x + w - 8.0, y + st.row_h / 2.0, ell(&fmt(c.1, &v), st.vsize, w - 12.0), st.vsize, fg, bold || c.0.is_none() && st.total_bold, Align::End));
        }
        s.line((0.0, rect.bottom()), (o.width, rect.bottom()), Line::solid(0.6, st.grid.with_alpha(140)));
        let point: Vec<(FieldRef, Scalar)> = rcols.iter().zip(path.iter()).filter_map(|(c, v)| d.cols.get(*c).map(|x| (x.field.clone(), v.clone()))).collect();
        let dimmed = dim(o, &point);
        let tip = vec![(row_label.clone(), path.iter().map(Scalar::text).collect::<Vec<_>>().join(" › "))];
        s.push_hit(Shape::Rect { rect: Rect::new(0.0, y, o.width, st.row_h), fill: None, stroke: None, radius: 0.0 }, Hit { point, tooltip: tip }, dimmed);
    }
    if nr == 0 {
        // Only columns/values: one row of totals.
        let y = body.y;
        for (k, c) in colspec.iter().enumerate() {
            let (Some(x), Some(w)) = (xs.get(k), widths.get(k)) else { continue };
            let v = match c.0 {
                Some(g) => cells.get(&(0, String::new(), groups.get(g).map(|x| key_of(x)).unwrap_or_default())).and_then(|r| r.get(c.1)).cloned().unwrap_or_default(),
                None => cells.get(&(0, String::new(), "\u{2}total".into())).and_then(|r| r.get(c.1)).cloned().unwrap_or_default(),
            };
            let v = if v.is_null() && nc == 0 { d.rows.first().and_then(|r| vcols.get(c.1).and_then(|vc| r.get(*vc))).cloned().unwrap_or_default() } else { v };
            s.push(text_shape(x + w - 8.0, y + st.row_h / 2.0, ell(&fmt(c.1, &v), st.vsize, w - 12.0), st.vsize, st.fg, false, Align::End));
        }
    }
    // Header.
    s.rect(Rect::new(0.0, 0.0, o.width, head_h), st.head_bg);
    s.push(text_shape(8.0, head_h - hh / 2.0, ell(&row_label, st.hsize, label_w - 12.0), st.hsize, st.head_fg, st.head_bold, Align::Start));
    for (k, c) in colspec.iter().enumerate() {
        let (Some(x), Some(w)) = (xs.get(k), widths.get(k)) else { continue };
        let vname = vcols.get(c.1).and_then(|x| d.cols.get(*x)).map(|x| x.label.clone()).unwrap_or_default();
        let gname = c.0.and_then(|g| groups.get(g)).map(|g| g.iter().map(|v| if v.is_null() { "(Blank)".into() } else { v.text() }).collect::<Vec<_>>().join(" ")).unwrap_or_else(|| "Total".into());
        if nc > 0 && nv > 1 {
            if c.1 == 0 {
                let span: f32 = widths.get(k..k + nv).map(|x| x.iter().sum()).unwrap_or(*w);
                s.push(text_shape(x + span / 2.0, hh / 2.0, ell(&gname, st.hsize, span - 10.0), st.hsize, st.head_fg, st.head_bold, Align::Center));
                s.line((x + 4.0, hh - 1.0), (x + span - 4.0, hh - 1.0), Line::solid(0.8, st.grid));
            }
            s.push(text_shape(x + w - 8.0, hh * 1.5, ell(&vname, st.hsize, w - 12.0), st.hsize, st.head_fg, false, Align::End));
        } else {
            let t = if nc > 0 { gname } else { vname };
            s.push(text_shape(x + w - 8.0, hh / 2.0, ell(&t, st.hsize, w - 12.0), st.hsize, st.head_fg, st.head_bold, Align::End));
        }
    }
    s.line((0.0, head_h), (o.width, head_h), Line::solid(1.2, st.head_line));
    // Grand total row.
    if show_grand {
        let ty = (body.y + content_h - offset).min(body.bottom()) + 1.0;
        s.rect(Rect::new(0.0, ty, o.width, st.row_h), st.total_bg);
        s.line((0.0, ty), (o.width, ty), Line::solid(1.2, st.head_line));
        s.push(text_shape(8.0, ty + st.row_h / 2.0, "Total".into(), st.vsize, st.fg, st.total_bold, Align::Start));
        for (k, c) in colspec.iter().enumerate() {
            let (Some(x), Some(w)) = (xs.get(k), widths.get(k)) else { continue };
            let v = cell_val(&[], c);
            let v = if v.is_null() && nc == 0 { d.totals.get(vcols.get(c.1).copied().unwrap_or(usize::MAX)).cloned().unwrap_or_default() } else { v };
            s.push(text_shape(x + w - 8.0, ty + st.row_h / 2.0, ell(&fmt(c.1, &v), st.vsize, w - 12.0), st.vsize, st.fg, st.total_bold, Align::End));
        }
    }
    if content_h > body.h + 1.0 {
        scrollbar(&mut s, o, body, content_h, offset);
    }
    s
}

/// Heat map and highlight table: rows × columns coloured by value.
pub fn heat(spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let Some(vi) = d.values().first().copied() else { return empty(o, "Add a field to Color") };
    let rc = d.first("rows").or_else(|| d.first("axis"));
    let cc = d.first("columns").or_else(|| d.first("legend"));
    let mut s = Scene::new(o.width, o.height);
    let rkeys: Vec<Scalar> = rc.map(|c| d.distinct(c)).unwrap_or_else(|| vec![Scalar::Null]);
    let ckeys: Vec<Scalar> = cc.map(|c| d.distinct(c)).unwrap_or_else(|| vec![Scalar::Null]);
    if d.rows.is_empty() {
        return empty(o, "No data for the current filters");
    }
    let mut grid: HashMap<(String, String), usize> = HashMap::new();
    for r in 0..d.rows.len() {
        let rk = rc.map(|c| d.cell(r, c).key()).unwrap_or_default();
        let ck = cc.map(|c| d.cell(r, c).key()).unwrap_or_default();
        grid.insert((rk, ck), r);
    }
    let vals: Vec<f64> = (0..d.rows.len()).filter_map(|r| d.num(r, vi)).filter(|v| v.is_finite()).collect();
    let lo = vals.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let diverging = lo < 0.0 && hi > 0.0;
    let color_for = |v: f64| -> Color {
        if diverging {
            let m = lo.abs().max(hi.abs());
            if v < 0.0 { o.theme.visual_background.lerp(o.theme.bad, (-v / m) * 0.85) } else { o.theme.visual_background.lerp(o.theme.series_color(0), (v / m) * 0.85) }
        } else {
            seq_color(o, if hi > lo { (v - lo) / (hi - lo) } else { 1.0 } * 0.9 + 0.1)
        }
    };
    let size = o.props.f32("values.size", 10.0) * 0.95;
    let row_text = |k: &Scalar| -> String {
        let r = rc.and_then(|c| (0..d.rows.len()).find(|r| d.cell(*r, c).key() == k.key()).map(|r| cat_text(d, r, c)));
        r.unwrap_or_default()
    };
    let col_text = |k: &Scalar| -> String { cc.and_then(|c| (0..d.rows.len()).find(|r| d.cell(*r, c).key() == k.key()).map(|r| cat_text(d, r, c))).unwrap_or_default() };
    let label_w = rkeys.iter().map(|k| text_width(&row_text(k), size)).fold(0.0, f32::max).min(o.width * 0.3) + 12.0;
    let clabels: Vec<String> = ckeys.iter().map(|k| abbreviate_months(&col_text(k))).collect();
    let cw = (o.width - label_w - 4.0) / ckeys.len().max(1) as f32;
    let maxw = clabels.iter().map(|t| text_width(t, size)).fold(0.0, f32::max);
    let rotate = maxw > cw - 4.0;
    let head = if rotate { (maxw * 0.7 + size).min(o.height * 0.3) } else { size * 2.0 };
    let legend_h = 22.0;
    let rh = ((o.height - head - legend_h) / rkeys.len().max(1) as f32).max(4.0);
    let fmt = ValueFmt::of_col(d, vi).uniform(lo.abs().max(hi.abs()));
    let highlight = spec.kind == VisualKind::HighlightTable;
    let all_fit = vals.iter().all(|v| text_width(&fmt.short(*v), size) < cw - 6.0) && rh > size * 1.4;
    let show_values = all_fit && (highlight || o.props.bool("labels.show", true));
    for (ci, t) in clabels.iter().enumerate() {
        let x = label_w + cw * (ci as f32 + 0.5);
        if rotate {
            s.push(Shape::Text { x: x + 3.0, y: head - 4.0, text: ell(t, size, head * 1.3), size, color: o.theme.foreground_muted, bold: false, italic: false, h: Align::Start, v: Align::Center, angle: -45.0 });
        } else {
            s.text(x, head / 2.0, ell(t, size, cw - 2.0), size, o.theme.foreground_muted, Align::Center, Align::Center);
        }
    }
    let gap = if highlight { 0.0 } else { 1.5 };
    for (ri, rk) in rkeys.iter().enumerate() {
        let y = head + rh * ri as f32;
        if rh >= size * 0.9 || ri % ((size / rh).ceil() as usize).max(1) == 0 {
            s.text(label_w - 6.0, y + rh / 2.0, ell(&row_text(rk), size, label_w - 8.0), size, o.theme.foreground_muted, Align::End, Align::Center);
        }
        for (ci, ck) in ckeys.iter().enumerate() {
            let x = label_w + cw * ci as f32;
            let rect = Rect::new(x + gap / 2.0, y + gap / 2.0, cw - gap, rh - gap);
            let Some(r) = grid.get(&(rk.key(), ck.key())).copied() else {
                if highlight {
                    s.push(Shape::Rect { rect, fill: None, stroke: Some(Line::solid(0.5, o.theme.visual_border)), radius: 0.0 });
                }
                continue;
            };
            let Some(v) = d.num(r, vi).filter(|v| v.is_finite()) else { continue };
            let c = o.props.point_colors.get(&color_key(d, r)).copied().unwrap_or_else(|| color_for(v));
            let h = hit(d, r, POINT_ROLES);
            let dimmed = dim(o, &h.point);
            s.push_hit(Shape::Rect { rect, fill: Some(c), stroke: if highlight { Some(Line::solid(0.5, o.theme.visual_background)) } else { None }, radius: if highlight { 0.0 } else { 2.0 } }, h, dimmed);
            let t = fmt.short(v);
            if show_values {
                s.text(x + cw / 2.0, y + rh / 2.0, t, size, c.contrast_text(), Align::Center, Align::Center);
            }
        }
    }
    // Colour scale: [min] ▬▬▬ [max], right-aligned.
    let y = o.height - 11.0;
    let hi_t = fmt.short(hi);
    let w = 110.0f32.min(o.width * 0.4);
    let x = o.width - text_width(&hi_t, 8.5) - 10.0 - w;
    for k in 0..24 {
        let t = k as f64 / 23.0;
        s.rect(Rect::new(x + w * k as f32 / 24.0, y - 4.0, w / 24.0 + 0.5, 8.0), color_for(lo + (hi - lo) * t));
    }
    s.text(x - 4.0, y, fmt.short(lo), 8.5, o.theme.foreground_muted, Align::End, Align::Center);
    s.text(x + w + 4.0, y, hi_t, 8.5, o.theme.foreground_muted, Align::Start, Align::Center);
    s
}
