//! Category charts on x/y axes: stacked / clustered / 100% bars and columns, lines, areas,
//! stacked areas, line + column combos (dual axis), ribbon, histogram, with legends, data and
//! total labels, small multiples and analytics overlays (reference lines, bands, trend,
//! forecast, error bars, anomalies).

use std::collections::HashMap;

use super::RenderOpts;
use super::axis::*;
use super::common::*;
use crate::analytics::Computed;
use crate::color::Color;
use crate::doc::*;
use crate::scalar::Scalar;
use crate::scene::{Align, Line, Rect, Scene, Shape, text_width};
use crate::vquery::VisualData;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Bar,
    Line,
    Area,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stack {
    None,
    Stacked,
    Percent,
}

#[derive(Clone, Copy, Debug)]
pub struct Cfg {
    pub horizontal: bool,
    pub stack: Stack,
    pub primary: Mark,
    pub secondary: Mark,
    pub ribbon: bool,
}

pub fn cfg(kind: VisualKind) -> Cfg {
    use VisualKind::*;
    let c = |horizontal, stack, primary| Cfg { horizontal, stack, primary, secondary: Mark::Line, ribbon: false };
    match kind {
        StackedBar => c(true, Stack::Stacked, Mark::Bar),
        ClusteredBar => c(true, Stack::None, Mark::Bar),
        StackedBar100 => c(true, Stack::Percent, Mark::Bar),
        StackedColumn | LineStackedColumn => c(false, Stack::Stacked, Mark::Bar),
        StackedColumn100 => c(false, Stack::Percent, Mark::Bar),
        Line | Sparkline => c(false, Stack::None, Mark::Line),
        Area => c(false, Stack::None, Mark::Area),
        StackedArea => c(false, Stack::Stacked, Mark::Area),
        StackedArea100 => c(false, Stack::Percent, Mark::Area),
        Ribbon => Cfg { ribbon: true, ..c(false, Stack::Stacked, Mark::Bar) },
        _ => c(false, Stack::None, Mark::Bar),
    }
}

/// One drawn series.
#[derive(Clone, Debug)]
pub struct Series {
    pub name: String,
    pub color: Color,
    pub mark: Mark,
    pub secondary: bool,
    pub col: usize,
    pub values: Vec<Option<f64>>,
    pub hl: Vec<Option<f64>>,
    pub rows: Vec<Option<usize>>,
    /// The legend value this series stands for (identity for legend clicks).
    pub legend: Option<(FieldRef, Scalar)>,
}

/// Data pivoted to categories × series.
#[derive(Clone, Debug, Default)]
pub struct Pivot {
    pub cats: Vec<String>,
    pub series: Vec<Series>,
    pub legend_title: String,
    pub fmt: ValueFmt,
    pub fmt2: ValueFmt,
}

/// The axis title of category columns ("Month" for a date level).
pub fn cat_title(d: &VisualData, cols: &[usize]) -> String {
    cols.iter()
        .filter_map(|c| d.cols.get(*c))
        .map(|c| match &c.field {
            FieldRef::Level { level, .. } => level.clone(),
            _ => c.label.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn pivot(d: &VisualData, o: &RenderOpts, rows: &[usize], cf: &Cfg) -> Pivot {
    let axis = d.role("axis");
    let legend = d.first("legend").filter(|i| d.cols.get(*i).is_some_and(|c| !c.is_value));
    let primaries: Vec<usize> = d.cols.iter().enumerate().filter(|(_, c)| c.is_value && c.role == "values").map(|(i, _)| i).collect();
    let primaries = if primaries.is_empty() { d.cols.iter().enumerate().filter(|(_, c)| c.is_value && !matches!(c.role.as_str(), "secondary" | "tooltips")).map(|(i, _)| i).collect() } else { primaries };
    let secondaries: Vec<usize> = d.cols.iter().enumerate().filter(|(_, c)| c.is_value && c.role == "secondary").map(|(i, _)| i).collect();
    let mut cat_index: HashMap<String, usize> = HashMap::new();
    let mut cats: Vec<String> = Vec::new();
    let mut row_cat: Vec<(usize, usize)> = Vec::new();
    for &r in rows {
        let key = axis.iter().map(|c| d.cell(r, *c).key()).collect::<Vec<_>>().join("\u{1}");
        let ci = *cat_index.entry(key).or_insert_with(|| {
            let label = if axis.is_empty() { String::new() } else { axis.iter().map(|c| cat_text(d, r, *c)).collect::<Vec<_>>().join(" ") };
            cats.push(label);
            cats.len() - 1
        });
        row_cat.push((r, ci));
    }
    if cats.is_empty() && !rows.is_empty() {
        cats.push(String::new());
    }
    let n = cats.len();
    let mut series: Vec<Series> = Vec::new();
    let mut skey: HashMap<String, usize> = HashMap::new();
    let has_hl = d.highlight.is_some();
    let mk = |name: String, col: usize, mark: Mark, secondary: bool, legend: Option<(FieldRef, Scalar)>| Series {
        name,
        color: Color::BLACK,
        mark,
        secondary,
        col,
        values: vec![None; n],
        hl: vec![None; n],
        rows: vec![None; n],
        legend,
    };
    for &(r, ci) in &row_cat {
        for &vc in &primaries {
            let (key, name, leg) = match legend {
                Some(lc) => {
                    let lv = d.cell(r, lc).clone();
                    let lname = cat_text(d, r, lc);
                    let name = if primaries.len() > 1 { format!("{} · {lname}", d.cols.get(vc).map(|c| c.label.as_str()).unwrap_or("")) } else { lname };
                    (format!("{vc}\u{1}{}", lv.key()), name, d.cols.get(lc).map(|c| (c.field.clone(), lv)))
                }
                None => (format!("{vc}"), d.cols.get(vc).map(|c| c.label.clone()).unwrap_or_default(), None),
            };
            let si = *skey.entry(key).or_insert_with(|| {
                series.push(mk(name, vc, cf.primary, false, leg));
                series.len() - 1
            });
            if let Some(s) = series.get_mut(si) {
                add(s, ci, d.num(r, vc), r, has_hl.then(|| hl_num(d, r, vc)).flatten());
            }
        }
    }
    for &vc in &secondaries {
        let mut s = mk(d.cols.get(vc).map(|c| c.label.clone()).unwrap_or_default(), vc, cf.secondary, true, None);
        for &(r, ci) in &row_cat {
            add(&mut s, ci, d.num(r, vc), r, has_hl.then(|| hl_num(d, r, vc)).flatten());
        }
        series.push(s);
    }
    for (i, s) in series.iter_mut().enumerate() {
        let key = match &s.legend {
            Some(l) => crate::fx::point_key(std::slice::from_ref(l)),
            None => format!("series:{}", s.name),
        };
        s.color = data_alpha(o, series_color(o, i, &key));
    }
    let legend_title = legend.and_then(|l| d.cols.get(l)).map(|c| c.label.clone()).unwrap_or_default();
    let fmt = primaries.first().map(|c| ValueFmt::of_col(d, *c)).unwrap_or_default().with_props(o, "yAxis");
    let fmt2 = secondaries.first().map(|c| ValueFmt::of_col(d, *c)).unwrap_or_default().with_props(o, "yAxis");
    Pivot { cats, series, legend_title, fmt, fmt2 }
}

fn add(s: &mut Series, ci: usize, v: Option<f64>, r: usize, hl: Option<f64>) {
    if let Some(v) = v.filter(|v| v.is_finite()) {
        if let Some(slot) = s.values.get_mut(ci) {
            *slot = Some(slot.unwrap_or(0.0) + v);
        }
    }
    if let Some(h) = hl.filter(|v| v.is_finite())
        && let Some(slot) = s.hl.get_mut(ci)
    {
        *slot = Some(slot.unwrap_or(0.0) + h);
    }
    if let Some(slot) = s.rows.get_mut(ci)
        && slot.is_none()
    {
        *slot = Some(r);
    }
}

/// Stacked extents (v0, v1) per series × category.
fn stack_extents(p: &Pivot, cf: &Cfg, secondary: bool) -> Vec<Vec<Option<(f64, f64)>>> {
    let n = p.cats.len();
    let mut out = vec![vec![None; n]; p.series.len()];
    let stacked = cf.stack != Stack::None;
    for ci in 0..n {
        let tot: f64 = p.series.iter().filter(|s| s.secondary == secondary && s.mark != Mark::Line).filter_map(|s| s.values.get(ci).copied().flatten()).map(f64::abs).sum();
        let mut pos = 0.0;
        let mut neg = 0.0;
        for (si, s) in p.series.iter().enumerate() {
            if s.secondary != secondary {
                continue;
            }
            let Some(v) = s.values.get(ci).copied().flatten() else { continue };
            let v = if cf.stack == Stack::Percent && s.mark != Mark::Line { if tot > 0.0 { v / tot } else { 0.0 } } else { v };
            let e = if stacked && s.mark != Mark::Line {
                if v >= 0.0 {
                    let e = (pos, pos + v);
                    pos += v;
                    e
                } else {
                    let e = (neg, neg + v);
                    neg += v;
                    e
                }
            } else {
                (0.0, v)
            };
            if let Some(slot) = out.get_mut(si).and_then(|r| r.get_mut(ci)) {
                *slot = Some(e);
            }
        }
    }
    out
}

fn domain(p: &Pivot, ext: &[Vec<Option<(f64, f64)>>], secondary: bool) -> (f64, f64, bool) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let mut has_bar = false;
    for (s, e) in p.series.iter().zip(ext.iter()) {
        if s.secondary != secondary {
            continue;
        }
        if s.mark != Mark::Line {
            has_bar = true;
        }
        for (a, b) in e.iter().flatten() {
            lo = lo.min(a.min(*b));
            hi = hi.max(a.max(*b));
        }
    }
    (lo, hi, has_bar)
}

/// Public entry: draws a cartesian chart (with small multiples when the "small" role is set).
pub fn render(spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let cf = cfg(spec.kind);
    let mut s = Scene::new(o.width, o.height);
    if d.values().is_empty() {
        return empty(o, "Add a field to Values / Y-axis");
    }
    let all: Vec<usize> = (0..d.rows.len()).collect();
    if d.rows.is_empty() {
        return empty(o, "No data for the current filters");
    }
    if let Some(sm) = d.first("small") {
        small_multiples(&mut s, spec, d, o, &cf, sm);
        return s;
    }
    let area = Rect::new(2.0, 2.0, (o.width - 4.0).max(1.0), (o.height - 4.0).max(1.0));
    draw(&mut s, spec, d, o, &cf, &all, area, None, true, o.analytics);
    s
}

fn small_multiples(s: &mut Scene, spec: &VisualSpec, d: &VisualData, o: &RenderOpts, cf: &Cfg, sm: usize) {
    let values = d.distinct(sm);
    let n = values.len().max(1);
    // Legend once, on top.
    let full: Vec<usize> = (0..d.rows.len()).collect();
    let p = pivot(d, o, &full, cf);
    let mut area = Rect::new(2.0, 2.0, o.width - 4.0, o.height - 4.0);
    if p.series.len() > 1 {
        let items: Vec<LegendItem> = p.series.iter().map(|x| LegendItem { label: x.name.clone(), color: x.color, line: x.mark == Mark::Line, hit: None, dimmed: false }).collect();
        area = legend(s, o, &items, &p.legend_title, area);
    }
    // Shared value domain.
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for v in &values {
        let rows: Vec<usize> = (0..d.rows.len()).filter(|r| d.cell(*r, sm).key() == v.key()).collect();
        let pp = pivot(d, o, &rows, cf);
        let ext = stack_extents(&pp, cf, false);
        let (a, b, _) = domain(&pp, &ext, false);
        lo = lo.min(a);
        hi = hi.max(b);
        groups.push(rows);
    }
    let aspect = area.w / area.h.max(1.0);
    let cols = ((n as f32 * aspect).sqrt().ceil() as usize).clamp(1, n);
    let rows_n = n.div_ceil(cols);
    let gap = 10.0;
    let pw = (area.w - gap * (cols as f32 - 1.0)) / cols as f32;
    let ph = (area.h - gap * (rows_n as f32 - 1.0)) / rows_n as f32;
    for (i, (v, rows)) in values.iter().zip(groups.iter()).enumerate() {
        let (cx, cy) = ((i % cols) as f32, (i / cols) as f32);
        let r = Rect::new(area.x + cx * (pw + gap), area.y + cy * (ph + gap), pw, ph);
        let title = if v.is_null() { "(Blank)".to_string() } else { rows.first().map(|r0| cat_text(d, *r0, sm)).unwrap_or_else(|| v.text()) };
        s.text_bold(r.x + 4.0, r.y + 2.0, ell(&title, 10.0, r.w - 8.0), 10.0, o.theme.foreground, Align::Start, Align::Start);
        let inner = Rect::new(r.x, r.y + 16.0, r.w, (r.h - 16.0).max(1.0));
        draw(s, spec, d, o, cf, rows, inner, Some((lo, hi)), false, &[]);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw(s: &mut Scene, spec: &VisualSpec, d: &VisualData, o: &RenderOpts, cf: &Cfg, rows: &[usize], area: Rect, forced: Option<(f64, f64)>, show_legend: bool, analytics: &[Computed]) {
    let mut p = pivot(d, o, rows, cf);
    let n = p.cats.len();
    if n == 0 || p.series.is_empty() {
        s.text(area.x + area.w / 2.0, area.y + area.h / 2.0, "No data", 11.0, o.theme.foreground_muted, Align::Center, Align::Center);
        return;
    }
    let sparkline = spec.kind == VisualKind::Sparkline;
    let mut area = area;
    // Legend.
    let has_legend_field = d.first("legend").is_some();
    if show_legend && !sparkline && (p.series.len() > 1 || has_legend_field) {
        let items: Vec<LegendItem> = p
            .series
            .iter()
            .map(|x| {
                let hit = x.legend.clone().map(|l| crate::scene::Hit { point: vec![l.clone()], tooltip: vec![(p.legend_title.clone(), x.name.clone())] });
                let dimmed = hit.as_ref().is_some_and(|h| dim(o, &h.point));
                LegendItem { label: x.name.clone(), color: x.color, line: x.mark == Mark::Line, hit, dimmed }
            })
            .collect();
        area = legend(s, o, &items, &p.legend_title, area);
    }
    let ext = stack_extents(&p, cf, false);
    let ext2 = stack_extents(&p, cf, true);
    let has_secondary = p.series.iter().any(|x| x.secondary);
    let sec_axis = has_secondary && o.props.bool("y2Axis.show", true);
    // Forecast slots extend the category domain (vertical charts).
    let mut slots = n;
    if !cf.horizontal {
        for a in analytics {
            if let Computed::Forecast { start_index, values, .. } = a {
                slots = slots.max(start_index + values.len());
            }
        }
    }
    // Primary domain.
    let (mut lo, mut hi, has_bar) = domain(&p, &ext, false);
    let (mut lo2, mut hi2, has_bar2) = domain(&p, &ext2, true);
    if !sec_axis && has_secondary {
        lo = lo.min(lo2);
        hi = hi.max(hi2);
    }
    if cf.stack != Stack::Percent {
        for a in analytics {
            match a {
                Computed::ValueLine { value, .. } => {
                    lo = lo.min(*value);
                    hi = hi.max(*value);
                }
                Computed::Band { from, to, .. } => {
                    lo = lo.min(*from);
                    hi = hi.max(*to);
                }
                Computed::Series { points, .. } => {
                    for (_, y) in points {
                        lo = lo.min(*y);
                        hi = hi.max(*y);
                    }
                }
                Computed::Forecast { lower, upper, .. } => {
                    for v in lower.iter().chain(upper.iter()).filter(|v| v.is_finite()) {
                        lo = lo.min(*v);
                        hi = hi.max(*v);
                    }
                }
                Computed::ErrorBars { low, high, .. } => {
                    for v in low.iter().chain(high.iter()).filter(|v| v.is_finite()) {
                        lo = lo.min(*v);
                        hi = hi.max(*v);
                    }
                }
                _ => {}
            }
        }
    }
    if let Some((a, b)) = forced {
        lo = lo.min(a);
        hi = hi.max(b);
    }
    if !lo.is_finite() || !hi.is_finite() {
        lo = 0.0;
        hi = 1.0;
    }
    if !lo2.is_finite() || !hi2.is_finite() {
        lo2 = 0.0;
        hi2 = 1.0;
    }
    let zero = |lo: f64, hi: f64, bar: bool| bar || (lo >= 0.0 && lo <= hi * 0.55) || (hi <= 0.0);
    let vlen = if cf.horizontal { area.w } else { area.h };
    let mut sc = if cf.stack == Stack::Percent { Scale::linear(lo.min(0.0), hi.max(1.0).min(1.0), true, 5) } else { Scale::from_props(o, "yAxis", lo, hi, zero(lo, hi, has_bar), tick_target(vlen * 0.8)) };
    let sc2 = Scale::linear(lo2, hi2, zero(lo2, hi2, has_bar2), tick_target(vlen * 0.8));
    let fmt = if cf.stack == Stack::Percent { ValueFmt::new("0%") } else { p.fmt.clone() };
    let fmt2 = p.fmt2.clone();
    if sparkline {
        sc = Scale::linear(lo, hi, false, 2);
    }
    // Axis text sizes / titles.
    let xsize = o.props.f32("xAxis.size", 10.0) * 0.95;
    let ysize = o.props.f32("yAxis.size", 10.0) * 0.92;
    let show_x = o.props.bool("xAxis.show", true) && !sparkline;
    let show_y = o.props.bool("yAxis.show", true) && !sparkline;
    let axis_cols = d.role("axis");
    let xt = {
        let t = o.props.text("xAxis.titleText", "");
        if t.is_empty() { cat_title(d, &axis_cols) } else { t }
    };
    let yt = {
        let t = o.props.text("yAxis.titleText", "");
        if t.is_empty() {
            let mut seen: Vec<String> = Vec::new();
            for x in p.series.iter().filter(|x| !x.secondary) {
                let l = d.cols.get(x.col).map(|c| c.label.clone()).unwrap_or_default();
                if !seen.contains(&l) {
                    seen.push(l);
                }
            }
            seen.join(" and ")
        } else {
            t
        }
    };
    let y2t = p.series.iter().filter(|x| x.secondary).map(|x| x.name.clone()).collect::<Vec<_>>().join(" and ");
    let x_title = show_x && o.props.bool("xAxis.title", true) && !xt.is_empty() && area.h > 120.0;
    let y_title = show_y && o.props.bool("yAxis.title", true) && !yt.is_empty() && area.w > 160.0;
    let tsize = 10.0;
    let muted = o.theme.foreground_muted;
    let labels_on = o.props.bool("labels.show", false);
    let lsize = o.props.f32("labels.size", 9.0);
    let lfmt = fmt.clone().with_props(o, "labels");
    let lfmt = if cf.stack == Stack::Percent { ValueFmt { format: "0.0%".into(), ..lfmt } } else { lfmt };

    // Plot rectangle.
    let plot;
    let cat_lay;
    if cf.horizontal {
        let label_w = if show_x { p.cats.iter().map(|c| text_width(c, xsize)).fold(0.0, f32::max).min(area.w * 0.32) + 10.0 } else { 4.0 };
        let left = area.x + label_w + if x_title { 16.0 } else { 0.0 };
        let bottom_h = if show_y { ysize * 1.7 } else { 2.0 } + if y_title { tsize * 1.6 } else { 0.0 };
        let right_pad = if labels_on { lfmt.short(hi).len() as f32 * lsize * 0.6 + 8.0 } else { 12.0 };
        plot = Rect::new(left, area.y + 2.0, (area.right() - left - right_pad).max(10.0), (area.h - bottom_h - 2.0).max(10.0));
        let band = plot.h / n as f32;
        cat_lay = CatLayout { step: ((xsize * 1.2) / band).ceil().max(1.0) as usize, angle: 0.0, extent: label_w, max_len: label_w - 8.0, size: xsize };
    } else {
        let yw = if show_y { value_axis_width(o, &sc, &fmt, "yAxis") } else { 4.0 };
        let y2w = if sec_axis && show_y { value_axis_width(o, &sc2, &fmt2, "yAxis") } else { 0.0 };
        let left = area.x + yw + if y_title { 16.0 } else { 0.0 };
        let right = area.right() - y2w - if sec_axis && y_title && !y2t.is_empty() { 16.0 } else { 0.0 } - if sec_axis { 0.0 } else { 8.0 };
        let pw = (right - left).max(10.0);
        let band = pw / slots as f32;
        if p.cats.iter().any(|c| text_width(c, xsize) > band * 0.92) {
            p.cats = p.cats.iter().map(|c| abbreviate_months(c)).collect();
        }
        let max_ext = area.h * o.props.f32("xAxis.maxHeight", 25.0).clamp(5.0, 50.0) / 100.0;
        let lay = if show_x { cat_layout(&p.cats, band, xsize, max_ext.max(xsize * 1.6)) } else { CatLayout { step: 1, angle: 0.0, extent: 2.0, max_len: band, size: xsize } };
        let top_pad = if labels_on || o.props.bool("totals.show", false) { lsize * 1.6 } else { ysize * 0.6 };
        let bottom = lay.extent + if x_title { tsize * 1.6 } else { 0.0 };
        plot = Rect::new(left, area.y + top_pad, pw, (area.h - bottom - top_pad).max(10.0));
        cat_lay = lay;
    }
    if let Some(bg) = o.props.color_opt("plot.background") {
        s.rect(plot, bg);
    }
    let band = if cf.horizontal { plot.h } else { plot.w } / slots.max(1) as f32;
    // Value → screen coordinate along the value axis.
    let vpos = |v: f64, sc: &Scale| -> f32 { if cf.horizontal { plot.x + plot.w * sc.frac(v) } else { plot.bottom() - plot.h * sc.frac(v) } };
    let cpos = |ci: f32| -> f32 { if cf.horizontal { plot.y + band * ci } else { plot.x + band * ci } };

    // Gridlines + value axis.
    if cf.horizontal {
        value_axis_h(s, o, &sc, plot, &fmt, "yAxis");
    } else if !sparkline {
        value_axis_v(s, o, &sc, plot, &fmt, "yAxis", false, true);
        if sec_axis {
            value_axis_v(s, o, &sc2, plot, &fmt2, "yAxis", true, false);
        }
        if o.props.bool("grid.vertical", false) {
            let gl = grid_line(o);
            for ci in 0..=slots {
                let x = cpos(ci as f32);
                s.line((x, plot.y), (x, plot.bottom()), gl.clone());
            }
        }
    }
    // Analytics bands (under the marks).
    for a in analytics {
        if let Computed::Band { from, to, label, color } = a {
            let (a0, a1) = (vpos(*from, &sc), vpos(*to, &sc));
            let r = if cf.horizontal { Rect::new(a0.min(a1), plot.y, (a1 - a0).abs(), plot.h) } else { Rect::new(plot.x, a0.min(a1), plot.w, (a1 - a0).abs()) };
            s.rect(r, *color);
            s.text(r.x + 4.0, r.y + 2.0, label, 9.0, color.with_alpha(255).shade(-0.3), Align::Start, Align::Start);
        }
    }

    // Bars.
    let bar_series: Vec<usize> = p.series.iter().enumerate().filter(|(_, x)| x.mark == Mark::Bar).map(|(i, _)| i).collect();
    let inner_pad = (o.props.f32("plot.innerPadding", 20.0).clamp(0.0, 90.0) / 100.0).max(if n == 1 { 0.4 } else { 0.0 });
    let group_w = band * (1.0 - inner_pad);
    let stacked = cf.stack != Stack::None;
    let k = if stacked { 1 } else { bar_series.len().max(1) };
    let spad = o.props.f32("plot.seriesPadding", 4.0).clamp(0.0, 50.0) / 100.0;
    let bw_full = group_w / k as f32;
    let bw = bw_full * (1.0 - if k > 1 { spad } else { 0.0 });
    let radius = o.props.f32("plot.cornerRadius", 2.0).min(bw / 2.0);
    let has_hl = d.highlight.is_some();
    let per_point = p.series.len() == 1 || o.props.bool("dataColors.showAll", false);
    let mut label_items: Vec<(f32, f32, String, Color, Align, Align)> = Vec::new();
    // Ribbon: bars ranked per category, connecting ribbons.
    let mut ribbon_rects: Vec<Vec<Option<Rect>>> = vec![vec![None; n]; p.series.len()];
    let ribbon_ext = if cf.ribbon { Some(ribbon_extents(&p)) } else { None };
    for (slot, &si) in bar_series.iter().enumerate() {
        let Some(sr) = p.series.get(si) else { continue };
        let sc_used = if sr.secondary && sec_axis { &sc2 } else { &sc };
        for ci in 0..n {
            let e = match &ribbon_ext {
                Some(re) => re.get(si).and_then(|r| r.get(ci)).copied().flatten(),
                None => ext.get(si).and_then(|r| r.get(ci)).copied().flatten().or_else(|| ext2.get(si).and_then(|r| r.get(ci)).copied().flatten()),
            };
            let Some((v0, v1)) = e else { continue };
            let off = if stacked { (band - group_w) / 2.0 } else { (band - group_w) / 2.0 + bw_full * slot as f32 + (bw_full - bw) / 2.0 };
            let c0 = cpos(ci as f32) + off;
            let base = if stacked || cf.ribbon { v0 } else { sc_used.base() };
            let (p0, p1) = (vpos(base, sc_used), vpos(v1, sc_used));
            let mut rect = if cf.horizontal { Rect::new(p0.min(p1), c0, (p1 - p0).abs(), bw) } else { Rect::new(c0, p0.min(p1), bw, (p1 - p0).abs()) };
            if stacked && p.series.len() > 1 {
                // 1px surface gap between stacked segments.
                if cf.horizontal && rect.w > 2.0 {
                    rect.w -= 1.0;
                } else if !cf.horizontal && rect.h > 2.0 {
                    rect.y += 1.0;
                    rect.h -= 1.0;
                }
            }
            if let Some(rr) = ribbon_rects.get_mut(si).and_then(|r| r.get_mut(ci)) {
                *rr = Some(rect);
            }
            let row = sr.rows.get(ci).copied().flatten();
            let mut color = sr.color;
            if per_point
                && let Some(r) = row
                && let Some(c) = o.props.point_colors.get(&color_key(d, r))
            {
                color = data_alpha(o, *c);
            }
            let Some(r) = row else { continue };
            let h = hit(d, r, POINT_ROLES);
            let dimmed = dim(o, &h.point);
            let rad = if stacked && p.series.len() > 1 { radius.min(1.5) } else { radius };
            if has_hl {
                s.push(Shape::Rect { rect, fill: Some(color.with_alpha(70)), stroke: None, radius: rad });
                if let Some(hv) = sr.hl.get(ci).copied().flatten() {
                    let full = sr.values.get(ci).copied().flatten().unwrap_or(0.0);
                    let hv = if cf.stack == Stack::Percent && full != 0.0 { (v1 - v0) * hv / full } else { hv };
                    let (q0, q1) = (vpos(if stacked { v0 } else { sc_used.base() }, sc_used), vpos(if stacked { v0 + hv } else { hv }, sc_used));
                    let hr = if cf.horizontal { Rect::new(q0.min(q1), rect.y, (q1 - q0).abs(), rect.h) } else { Rect::new(rect.x, q0.min(q1), rect.w, (q1 - q0).abs()) };
                    s.push_hit(Shape::Rect { rect: hr, fill: Some(color), stroke: None, radius: rad }, h.clone(), false);
                }
                // Hit area for the whole bar.
                s.push_hit(Shape::Rect { rect, fill: None, stroke: None, radius: 0.0 }, h, false);
            } else {
                s.push_hit(Shape::Rect { rect, fill: Some(color), stroke: None, radius: rad }, h, dimmed);
            }
            if labels_on {
                let raw = sr.values.get(ci).copied().flatten().unwrap_or(v1 - v0);
                let shown = if cf.stack == Stack::Percent { v1 - v0 } else { raw };
                let tot: f64 = p.series.iter().filter(|x| x.mark == Mark::Bar).filter_map(|x| x.values.get(ci).copied().flatten()).sum();
                let pct = if tot != 0.0 { format!("{:.1}%", raw / tot * 100.0) } else { String::new() };
                let text = label_template(o, &lfmt.short(shown), p.cats.get(ci).map(String::as_str).unwrap_or(""), &sr.name, &pct);
                let tw = text_width(&text, lsize);
                let pos = o.props.text("labels.position", "Auto");
                let neg = v1 < v0 || (!stacked && v1 < 0.0);
                if cf.horizontal {
                    let fits_in = rect.w > tw + 8.0 && rect.h >= lsize;
                    let outside = !stacked && (pos == "Outside end" || pos == "Auto") && (if neg { rect.x - tw - 4.0 > plot.x - 2.0 } else { rect.right() + tw + 4.0 < plot.right() + 40.0 });
                    let cy = rect.y + rect.h / 2.0;
                    if outside {
                        if neg {
                            label_items.push((rect.x - 4.0, cy, text, label_color(o, None), Align::End, Align::Center));
                        } else {
                            label_items.push((rect.right() + 4.0, cy, text, label_color(o, None), Align::Start, Align::Center));
                        }
                    } else if fits_in {
                        let (x, h) = match pos.as_str() {
                            "Inside base" => (rect.x + 4.0, Align::Start),
                            "Inside center" => (rect.x + rect.w / 2.0, Align::Center),
                            _ if stacked => (rect.x + rect.w / 2.0, Align::Center),
                            _ => (rect.right() - 4.0, Align::End),
                        };
                        label_items.push((x, cy, text, label_color(o, Some(color)), h, Align::Center));
                    }
                } else {
                    let fits_in = rect.h > lsize + 4.0 && rect.w > tw + 2.0;
                    let room_above = rect.y - plot.y + plot.y - area.y > lsize + 2.0;
                    let outside = !stacked && (pos == "Outside end" || pos == "Auto") && (neg || room_above) && tw <= band * 1.05;
                    let cx = rect.x + rect.w / 2.0;
                    if outside {
                        if neg {
                            label_items.push((cx, rect.bottom() + 2.0, text, label_color(o, None), Align::Center, Align::Start));
                        } else {
                            label_items.push((cx, rect.y - 2.0, text, label_color(o, None), Align::Center, Align::End));
                        }
                    } else if fits_in {
                        let (y, v) = match pos.as_str() {
                            "Inside base" => (rect.bottom() - 3.0, Align::End),
                            "Inside center" => (rect.y + rect.h / 2.0, Align::Center),
                            _ if stacked => (rect.y + rect.h / 2.0, Align::Center),
                            _ => (rect.y + 3.0, Align::Start),
                        };
                        label_items.push((cx, y, text, label_color(o, Some(color)), Align::Center, v));
                    }
                }
            }
        }
    }
    // Ribbons between consecutive categories.
    if cf.ribbon {
        let mut ribbons = Vec::new();
        for (si, rects) in ribbon_rects.iter().enumerate() {
            let color = p.series.get(si).map(|x| x.color).unwrap_or(o.theme.accent);
            for ci in 1..n {
                if let (Some(Some(a)), Some(Some(b))) = (rects.get(ci - 1), rects.get(ci)) {
                    ribbons.push(Shape::Path { points: vec![(a.right(), a.y), (b.x, b.y), (b.x, b.bottom()), (a.right(), a.bottom())], closed: true, fill: Some(color.with_alpha(70)), stroke: None });
                }
            }
        }
        // Ribbons go under the bars: insert them before the bars just drawn.
        let first_bar = s.items.iter().position(|it| it.hit.is_some()).unwrap_or(s.items.len());
        for (i, r) in ribbons.into_iter().enumerate() {
            s.items.insert(first_bar + i, crate::scene::Item { shape: r, hit: None, dimmed: false });
        }
    }
    // Total labels on stacks.
    if o.props.bool("totals.show", false) && stacked && cf.stack != Stack::Percent && !bar_series.is_empty() {
        let tfmt = fmt.clone().with_props(o, "labels");
        for ci in 0..n {
            let tot: f64 = bar_series.iter().filter_map(|si| p.series.get(*si)).filter(|x| !x.secondary).filter_map(|x| x.values.get(ci).copied().flatten()).sum();
            let top = bar_series.iter().filter_map(|si| ext.get(*si).and_then(|r| r.get(ci)).copied().flatten()).map(|(a, b)| a.max(b)).fold(0.0f64, f64::max);
            let t = tfmt.short(tot);
            let c = cpos(ci as f32) + band / 2.0;
            if cf.horizontal {
                label_items.push((vpos(top, &sc) + 4.0, c, t, o.theme.foreground, Align::Start, Align::Center));
            } else if text_width(&t, lsize) < band * 1.1 {
                label_items.push((c, vpos(top, &sc) - 2.0, t, o.theme.foreground, Align::Center, Align::End));
            }
        }
    }
    // Areas and lines.
    let lw = o.props.f32("lines.width", if sparkline { 2.0 } else { 2.5 });
    let lstyle = o.props.text("lines.style", "solid");
    let area_alpha = ((100.0 - o.props.f32("area.transparency", 60.0).clamp(0.0, 100.0)) / 100.0 * 255.0) as u8;
    let markers = o.props.bool("markers.show", false);
    let msize = o.props.f32("markers.size", 4.0);
    let mshape = o.props.text("markers.shape", "Circle");
    let center = |ci: usize| cpos(ci as f32) + band / 2.0;
    let line_series: Vec<usize> = p.series.iter().enumerate().filter(|(_, x)| x.mark != Mark::Bar).map(|(i, _)| i).collect();
    // Fills first (so lines sit on top).
    for &si in &line_series {
        let Some(sr) = p.series.get(si) else { continue };
        if sr.mark != Mark::Area {
            continue;
        }
        let sc_used = if sr.secondary && sec_axis { &sc2 } else { &sc };
        let ex = if sr.secondary { &ext2 } else { &ext };
        for run in runs(ex.get(si).map(|v| v.as_slice()).unwrap_or(&[])) {
            let top: Vec<(f32, f32)> = run.iter().map(|(ci, (_, v1))| (center(*ci), vpos(*v1, sc_used))).collect();
            let bot: Vec<(f32, f32)> = run.iter().rev().map(|(ci, (v0, _))| (center(*ci), vpos(if stacked { *v0 } else { sc_used.base() }, sc_used))).collect();
            let mut poly = interpolate(o, &top);
            let mut b2 = interpolate(o, &bot);
            poly.append(&mut b2);
            if poly.len() >= 3 {
                s.push(Shape::Path { points: poly, closed: true, fill: Some(sr.color.with_alpha(area_alpha)), stroke: None });
            }
        }
    }
    let mut last_label_x: HashMap<usize, f32> = HashMap::new();
    for &si in &line_series {
        let Some(sr) = p.series.get(si) else { continue };
        let sc_used = if sr.secondary && sec_axis { &sc2 } else { &sc };
        let ex = if sr.secondary { &ext2 } else { &ext };
        let line = Line::from_style(lw, sr.color, &lstyle);
        for run in runs(ex.get(si).map(|v| v.as_slice()).unwrap_or(&[])) {
            let pts: Vec<(f32, f32)> = run.iter().map(|(ci, (_, v1))| (center(*ci), vpos(*v1, sc_used))).collect();
            if pts.len() >= 2 {
                s.push(Shape::Path { points: interpolate(o, &pts), closed: false, fill: None, stroke: Some(line.clone()) });
            } else if let Some((x, y)) = pts.first() {
                s.push(Shape::Circle { cx: *x, cy: *y, r: lw.max(2.5), fill: Some(sr.color), stroke: None });
            }
        }
        // Markers / hit targets / labels.
        let ex_s = ex.get(si);
        for ci in 0..n {
            let Some((_, v1)) = ex_s.and_then(|v| v.get(ci)).copied().flatten() else { continue };
            let Some(r) = sr.rows.get(ci).copied().flatten() else { continue };
            let (x, y) = (center(ci), vpos(v1, sc_used));
            let h = hit(d, r, POINT_ROLES);
            let dimmed = dim(o, &h.point);
            if markers || n == 1 {
                s.push_hit(marker_shape(&mshape, x, y, msize.max(2.0), sr.color, Some(o.theme.visual_background)), h, dimmed);
            } else {
                s.push_hit(Shape::Circle { cx: x, cy: y, r: (band / 2.0).clamp(4.0, 10.0), fill: None, stroke: None }, h, dimmed);
            }
            if labels_on {
                let raw = sr.values.get(ci).copied().flatten().unwrap_or(v1);
                let text = label_template(o, &(if sr.secondary { fmt2.clone().with_props(o, "labels") } else { lfmt.clone() }).short(if cf.stack == Stack::Percent { v1 } else { raw }), p.cats.get(ci).map(String::as_str).unwrap_or(""), &sr.name, "");
                let tw = text_width(&text, lsize);
                let prev = last_label_x.get(&si).copied().unwrap_or(f32::MIN);
                if x - tw / 2.0 > prev + 3.0 {
                    label_items.push((x, y - msize - 3.0, text, label_color(o, None), Align::Center, Align::End));
                    last_label_x.insert(si, x + tw / 2.0);
                }
            }
        }
    }
    // Sparkline: highlight last / min / max.
    if sparkline {
        if let Some(sr) = p.series.first() {
            let vals: Vec<(usize, f64)> = sr.values.iter().enumerate().filter_map(|(i, v)| v.map(|v| (i, v))).collect();
            if let Some((li, lv)) = vals.last() {
                s.push(Shape::Circle { cx: center(*li), cy: vpos(*lv, &sc), r: 3.0, fill: Some(sr.color), stroke: Some(Line::solid(1.5, o.theme.visual_background)) });
            }
        }
        return;
    }
    // Analytics lines on top.
    let primary_col = p.series.iter().find(|x| !x.secondary).map(|x| x.col);
    let cat_series = primary_col.map(|c| crate::analytics::category_values(d, c)).unwrap_or_default();
    for a in analytics {
        match a {
            Computed::ValueLine { value, label, color, width, dash } => {
                let v = vpos(*value, &sc);
                let st = Line::from_style(*width, *color, dash);
                let txt = format!("{label} {}", fmt.short(*value));
                if cf.horizontal {
                    s.line((v, plot.y), (v, plot.bottom()), st);
                    s.text(v + 3.0, plot.y + 1.0, txt, 9.0, *color, Align::Start, Align::Start);
                } else {
                    s.line((plot.x, v), (plot.right(), v), st);
                    label_box(s, o, plot.right() - 2.0, v - 2.0, &txt, 9.0, *color, Align::End, Align::End);
                }
            }
            Computed::Series { points, label, color, width, dash } => {
                if cf.horizontal {
                    continue;
                }
                let pts: Vec<(f32, f32)> = points.iter().map(|(x, y)| (plot.x + band * (*x as f32 + 0.5), vpos(*y, &sc))).collect();
                if pts.len() >= 2 {
                    s.push(Shape::Path { points: pts.clone(), closed: false, fill: None, stroke: Some(Line::from_style(*width, *color, dash)) });
                    if let Some((x, y)) = pts.last() {
                        s.text(*x, *y - 4.0, label, 9.0, *color, Align::End, Align::End);
                    }
                }
            }
            Computed::Forecast { start_index, values, lower, upper, color } => {
                if cf.horizontal {
                    continue;
                }
                let fc = if *color == Color::rgb(0x55, 0x55, 0x55) { p.series.first().map(|x| x.color).unwrap_or(*color) } else { *color };
                let x_of = |i: usize| plot.x + band * (i as f32 + 0.5);
                let mut up: Vec<(f32, f32)> = upper.iter().enumerate().map(|(i, v)| (x_of(start_index + i), vpos(*v, &sc))).collect();
                let mut lowp: Vec<(f32, f32)> = lower.iter().enumerate().map(|(i, v)| (x_of(start_index + i), vpos(*v, &sc))).collect();
                let mut line_pts: Vec<(f32, f32)> = values.iter().enumerate().map(|(i, v)| (x_of(start_index + i), vpos(*v, &sc))).collect();
                if *start_index > 0
                    && let Some(prev) = cat_series.get(start_index - 1).copied().filter(|v| v.is_finite())
                {
                    let pp = (x_of(start_index - 1), vpos(prev, &sc));
                    line_pts.insert(0, pp);
                    up.insert(0, pp);
                    lowp.insert(0, pp);
                }
                lowp.reverse();
                up.extend(lowp);
                if up.len() >= 3 {
                    s.push(Shape::Path { points: up, closed: true, fill: Some(fc.with_alpha(38)), stroke: None });
                }
                let first_fc = line_pts.first().map(|p| p.0).unwrap_or(plot.x);
                s.line((first_fc, plot.y), (first_fc, plot.bottom()), Line::dashed(1.0, muted.with_alpha(90)));
                if line_pts.len() >= 2 {
                    s.push(Shape::Path { points: interpolate(o, &line_pts), closed: false, fill: None, stroke: Some(Line { width: lw.max(2.0), color: fc, dash: Some((5.0, 3.0)) }) });
                }
                s.text(first_fc + 4.0, plot.y + 1.0, "Forecast", 9.0, muted, Align::Start, Align::Start);
            }
            Computed::ErrorBars { low, high, color } => {
                for ci in 0..n {
                    let (Some(a), Some(b)) = (low.get(ci).filter(|v| v.is_finite()), high.get(ci).filter(|v| v.is_finite())) else { continue };
                    let c = center(ci);
                    let (pa, pb) = (vpos(*a, &sc), vpos(*b, &sc));
                    let w = (bw * 0.25).clamp(2.0, 6.0);
                    let st = Line::solid(1.2, *color);
                    if cf.horizontal {
                        s.line((pa, c), (pb, c), st.clone());
                        s.line((pa, c - w), (pa, c + w), st.clone());
                        s.line((pb, c - w), (pb, c + w), st);
                    } else {
                        s.line((c, pa), (c, pb), st.clone());
                        s.line((c - w, pa), (c + w, pa), st.clone());
                        s.line((c - w, pb), (c + w, pb), st);
                    }
                }
            }
            Computed::Anomalies { rows: idx, color } => {
                for ci in idx {
                    let Some(v) = cat_series.get(*ci).copied().filter(|v| v.is_finite()) else { continue };
                    let (x, y) = if cf.horizontal { (vpos(v, &sc), center(*ci)) } else { (center(*ci), vpos(v, &sc)) };
                    s.push(Shape::Circle { cx: x, cy: y, r: 6.0, fill: Some(color.with_alpha(40)), stroke: Some(Line::solid(1.8, *color)) });
                }
            }
            _ => {}
        }
    }
    // Data labels on top of everything.
    for (x, y, t, c, h, v) in label_items {
        label_box(s, o, x, y, &t, lsize, c, h, v);
    }
    // Zero baseline.
    if !sc.log && sc.lo < 0.0 && sc.hi > 0.0 || has_bar {
        let z = vpos(sc.base(), &sc);
        if cf.horizontal {
            baseline(s, o, (z, plot.y), (z, plot.bottom()));
        } else {
            baseline(s, o, (plot.x, z), (plot.right(), z));
        }
    }
    // Category labels.
    let xcol = axis_text(o, "xAxis");
    if show_x {
        for (ci, c) in p.cats.iter().enumerate() {
            if ci % cat_lay.step.max(1) != 0 {
                continue;
            }
            if cf.horizontal {
                let y = cpos(ci as f32) + band / 2.0;
                s.text(plot.x - 6.0, y, ell(c, cat_lay.size, cat_lay.max_len), cat_lay.size, xcol, Align::End, Align::Center);
            } else {
                draw_cat_label(s, &cat_lay, center(ci), plot.bottom(), c, xcol);
            }
        }
    }
    // Axis titles.
    if cf.horizontal {
        if x_title {
            vertical_title(s, area.x + 6.0, plot.y + plot.h / 2.0, &xt, tsize, muted, plot.h);
        }
        if y_title {
            s.text(plot.x + plot.w / 2.0, area.bottom() - 1.0, ell(&yt, tsize, plot.w), tsize, muted, Align::Center, Align::End);
        }
    } else {
        if x_title {
            s.text(plot.x + plot.w / 2.0, area.bottom() - 1.0, ell(&xt, tsize, plot.w), tsize, muted, Align::Center, Align::End);
        }
        if y_title {
            vertical_title(s, area.x + 6.0, plot.y + plot.h / 2.0, &yt, tsize, muted, plot.h);
            if sec_axis && !y2t.is_empty() {
                vertical_title(s, area.right() - 6.0, plot.y + plot.h / 2.0, &y2t, tsize, muted, plot.h);
            }
        }
    }
}

/// Runs of consecutive categories with values (lines break at gaps).
fn runs(ex: &[Option<(f64, f64)>]) -> Vec<Vec<(usize, (f64, f64))>> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    for (i, e) in ex.iter().enumerate() {
        match e {
            Some(v) => cur.push((i, *v)),
            None => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Ribbon chart: per category, series stacked by rank (largest at the top).
fn ribbon_extents(p: &Pivot) -> Vec<Vec<Option<(f64, f64)>>> {
    let n = p.cats.len();
    let mut out = vec![vec![None; n]; p.series.len()];
    for ci in 0..n {
        let mut idx: Vec<(usize, f64)> = p.series.iter().enumerate().filter(|(_, x)| x.mark == Mark::Bar).filter_map(|(i, x)| x.values.get(ci).copied().flatten().map(|v| (i, v.max(0.0)))).collect();
        idx.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let mut acc = 0.0;
        for (i, v) in idx {
            if let Some(slot) = out.get_mut(i).and_then(|r| r.get_mut(ci)) {
                *slot = Some((acc, acc + v));
            }
            acc += v;
        }
    }
    out
}

/// Histogram: bins a numeric axis (or uses the bins vquery made) into counts.
pub fn histogram(spec: &VisualSpec, d: &VisualData, o: &RenderOpts) -> Scene {
    let Some(ax) = d.first("axis") else { return empty(o, "Add a numeric field to Value (binned)") };
    let vi = d.values().first().copied();
    let pts: Vec<(f64, f64)> = (0..d.rows.len()).filter_map(|r| Some((d.num(r, ax)?, vi.and_then(|v| d.num(r, v)).unwrap_or(1.0)))).collect();
    if pts.is_empty() {
        // Text axis: fall back to a column chart of counts.
        return render(&VisualSpec { kind: VisualKind::ClusteredColumn, ..spec.clone() }, d, o);
    }
    let lo = pts.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let hi = pts.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let binned = d.cols.get(ax).is_some_and(|c| c.label.ends_with("(bins)"));
    let distinct = d.distinct(ax).len();
    let (width, nb) = if binned || distinct <= 12 {
        let mut xs: Vec<f64> = pts.iter().map(|p| p.0).collect();
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        xs.dedup();
        let w = xs.windows(2).filter_map(|w| w.get(1).zip(w.first()).map(|(b, a)| b - a)).fold(f64::INFINITY, f64::min);
        let w = if w.is_finite() && w > 0.0 { w } else { 1.0 };
        (w, (((hi - lo) / w).round() as usize + 1).min(200))
    } else {
        let target = o.props.num("bins.count", (pts.len() as f64).sqrt().clamp(5.0, 30.0)).max(1.0);
        let w = nice_step((hi - lo).max(1e-9) / target);
        (w, (((hi - lo) / w).floor() as usize + 1).min(200))
    };
    let start = if binned || distinct <= 12 { lo } else { (lo / width).floor() * width };
    let mut counts = vec![0.0; nb.max(1)];
    for (x, w) in &pts {
        let b = (((x - start) / width).floor().max(0.0) as usize).min(nb.saturating_sub(1));
        if let Some(c) = counts.get_mut(b) {
            *c += w;
        }
    }
    let fmt = d.cols.get(ax).map(|c| ValueFmt::new(&c.format)).unwrap_or_default();
    let label = d.cols.get(ax).map(|c| c.label.clone()).unwrap_or_default();
    let vlabel = vi.and_then(|v| d.cols.get(v)).map(|c| c.label.clone()).unwrap_or_else(|| "Count".into());
    let mut hd = VisualData::default();
    hd.cols.push(crate::vquery::DataCol { label: label.clone(), role: "axis".into(), field: d.cols.get(ax).map(|c| c.field.clone()).unwrap_or(FieldRef::col("", "")), is_value: false, format: String::new(), col_type: crate::build::ColType::Text, show_as: ShowAs::Value });
    hd.cols.push(crate::vquery::DataCol { label: vlabel, role: "values".into(), field: FieldRef::measure("", "Count"), is_value: true, format: "#,0".into(), col_type: crate::build::ColType::Number, show_as: ShowAs::Value });
    for (i, c) in counts.iter().enumerate() {
        let a = start + width * i as f64;
        let name = if binned || distinct <= 12 { fmt.short(a) } else { format!("{}–{}", fmt.short(a), fmt.short(a + width)) };
        hd.rows.push(vec![Scalar::Text(name), Scalar::Num(*c)]);
    }
    let mut props = o.props.clone();
    props.map.entry("plot.innerPadding".into()).or_insert(Prop::Num(4.0));
    if !props.map.contains_key("plot.innerPadding") || props.num("plot.innerPadding", 20.0) == 20.0 {
        props.map.insert("plot.innerPadding".into(), Prop::Num(4.0));
    }
    let o2 = RenderOpts { props: &props, analytics: &[], ..*o };
    render(&VisualSpec { kind: VisualKind::ClusteredColumn, ..spec.clone() }, &hd, &o2)
}
