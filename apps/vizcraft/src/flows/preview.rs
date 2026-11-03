//! Previews: a virtualized read-only table grid and the node summaries (metric cards, scatter
//! plots, bar charts, heatmaps, logs).

use egui::{Align2, Color32, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use vizcraft_core::build::{ColType, TableData};
use vizcraft_core::flow::{Bars, Scatter, Summary};
use vizcraft_core::scalar::Scalar;

use crate::theme::Tokens;

pub const PALETTE: [Color32; 10] = [
    Color32::from_rgb(0x2B, 0x6C, 0xE0),
    Color32::from_rgb(0xF5, 0x9E, 0x0B),
    Color32::from_rgb(0x10, 0xB9, 0x81),
    Color32::from_rgb(0xEF, 0x44, 0x44),
    Color32::from_rgb(0x8B, 0x5C, 0xF6),
    Color32::from_rgb(0x06, 0xB6, 0xD4),
    Color32::from_rgb(0xEC, 0x48, 0x99),
    Color32::from_rgb(0x84, 0xCC, 0x16),
    Color32::from_rgb(0x64, 0x74, 0x8B),
    Color32::from_rgb(0xF9, 0x73, 0x16),
];

pub fn cell_text(v: &Scalar, ty: ColType) -> String {
    match v {
        Scalar::Null => String::new(),
        Scalar::Num(n) if ty.is_temporal() => vizcraft_core::numfmt::format_date(*n, if ty == ColType::Date || n.fract() == 0.0 { "yyyy-mm-dd" } else { "yyyy-mm-dd hh:mm" }),
        Scalar::Num(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                vizcraft_core::numfmt::format(*n, if ty == ColType::Integer && n.abs() >= 10000.0 { "#,0" } else { "0" })
            } else if n.abs() >= 1000.0 {
                vizcraft_core::numfmt::format(*n, "#,0.##")
            } else {
                vizcraft_core::numfmt::format(*n, "0.####")
            }
        }
        Scalar::Bool(b) => if *b { "true".into() } else { "false".into() },
        Scalar::Text(t) => t.clone(),
    }
}

fn type_badge(ty: ColType) -> &'static str {
    match ty {
        ColType::Text => "Abc",
        ColType::Number => "1.2",
        ColType::Integer => "123",
        ColType::Date | ColType::DateTime => "Date",
        ColType::Time => "Time",
        ColType::Bool => "T/F",
        ColType::Any => "Any",
    }
}

/// A read-only table grid (virtualized rows, sticky header, auto column widths).
pub fn data_grid(ui: &mut Ui, t: &Tokens, data: &TableData, id: impl std::hash::Hash + std::fmt::Debug, max_height: f32) {
    let rh = 22.0;
    let head_h = 40.0;
    let num_w = 46.0;
    let font = FontId::proportional(12.0);
    // Column widths from the header and a sample of values.
    let widths: Vec<f32> = data
        .columns
        .iter()
        .enumerate()
        .map(|(c, name)| {
            let ty = data.types.get(c).copied().unwrap_or_default();
            let mut w = crate::widgets::text_w(ui, name) + 26.0;
            for r in data.rows.iter().take(60) {
                let s = cell_text(r.get(c).unwrap_or(&Scalar::Null), ty);
                w = w.max(crate::widgets::text_w(ui, &s) + 18.0);
            }
            w.clamp(64.0, 260.0)
        })
        .collect();
    let total_w: f32 = num_w + widths.iter().sum::<f32>();
    let n = data.rows.len();
    let height = max_height.min(head_h + rh * n.max(1) as f32 + 16.0).max(80.0);
    let (outer, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    ui.painter().rect(outer, 6.0, t.panel, Stroke::new(1.0, t.border), StrokeKind::Inside);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(outer.shrink(1.0)).id_salt(("data_grid", id)));
    egui::ScrollArea::both().auto_shrink([false, false]).show_viewport(&mut child, |ui, vp| {
        let origin = ui.max_rect().min;
        ui.allocate_rect(Rect::from_min_size(origin, vec2(total_w, head_h + rh * n as f32)), Sense::hover());
        let p = ui.painter().clone();
        let clip = ui.clip_rect();
        let i0 = ((vp.min.y - head_h).max(0.0) / rh).floor() as usize;
        let i1 = (((vp.max.y - head_h).max(0.0) / rh).ceil() as usize + 1).min(n);
        let font2 = font.clone();
        for i in i0..i1 {
            let y = origin.y + head_h + i as f32 * rh;
            if i % 2 == 1 {
                p.rect_filled(Rect::from_min_size(pos2(clip.left(), y), vec2(total_w.max(clip.width()), rh)), 0.0, t.panel_alt);
            }
            p.text(pos2(origin.x + num_w - 8.0, y + rh / 2.0), Align2::RIGHT_CENTER, (i + 1).to_string(), FontId::proportional(10.5), t.muted);
            let mut x = origin.x + num_w;
            let Some(row) = data.rows.get(i) else { continue };
            for (c, w) in widths.iter().enumerate() {
                if x > clip.right() {
                    break;
                }
                if x + w >= clip.left() {
                    let ty = data.types.get(c).copied().unwrap_or_default();
                    let v = row.get(c).unwrap_or(&Scalar::Null);
                    let s = cell_text(v, ty);
                    let cell = Rect::from_min_size(pos2(x, y), vec2(*w, rh));
                    let cp = p.with_clip_rect(cell.shrink2(vec2(3.0, 0.0)).intersect(clip));
                    if matches!(v, Scalar::Num(_)) && !ty.is_temporal() {
                        cp.text(cell.right_center() - vec2(8.0, 0.0), Align2::RIGHT_CENTER, s, font2.clone(), t.text);
                    } else if v.is_null() {
                        cp.text(cell.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, "null", FontId::proportional(11.0), t.border);
                    } else {
                        cp.text(cell.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, s, font2.clone(), t.text);
                    }
                }
                x += w;
            }
        }
        // Sticky header.
        let hy = clip.top();
        let head = Rect::from_min_size(pos2(clip.left(), hy), vec2(clip.width(), head_h));
        p.rect_filled(head, 0.0, t.panel_alt);
        p.line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0, t.border));
        let mut x = origin.x + num_w;
        for (c, w) in widths.iter().enumerate() {
            let ty = data.types.get(c).copied().unwrap_or_default();
            let cell = Rect::from_min_size(pos2(x, hy), vec2(*w, head_h));
            let cp = p.with_clip_rect(cell.shrink2(vec2(3.0, 0.0)).intersect(clip));
            cp.text(pos2(cell.left() + 8.0, hy + 13.0), Align2::LEFT_CENTER, data.columns.get(c).cloned().unwrap_or_default(), FontId::proportional(12.0), t.text);
            cp.text(pos2(cell.left() + 8.0, hy + 29.0), Align2::LEFT_CENTER, type_badge(ty), FontId::proportional(10.0), t.accent);
            p.line_segment([cell.right_top() + vec2(0.0, 8.0), cell.right_bottom() - vec2(0.0, 8.0)], Stroke::new(1.0, t.border));
            x += w;
        }
        p.rect_filled(Rect::from_min_size(head.min, vec2(num_w.min(head.width()), head_h)), 0.0, t.panel_alt);
    });
}

/// Metric cards in a wrapping row.
pub fn metric_cards(ui: &mut Ui, t: &Tokens, metrics: &[(String, String)]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        for (k, v) in metrics {
            let long = v.len() > 22;
            egui::Frame::new().fill(t.panel_alt).stroke(Stroke::new(1.0, t.border)).corner_radius(8.0).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                ui.set_max_width(if long { 360.0 } else { 220.0 });
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(RichText::new(k).size(10.5).color(t.muted));
                    ui.label(RichText::new(v).size(if long { 12.0 } else { 17.0 }).strong().color(t.text));
                });
            });
        }
    });
}

fn nice_ticks(lo: f64, hi: f64, n: usize) -> Vec<f64> {
    if !(hi > lo) {
        return vec![lo];
    }
    let raw = (hi - lo) / n.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0].iter().map(|m| m * mag).find(|s| *s >= raw).unwrap_or(raw);
    let start = (lo / step).ceil() * step;
    let mut v = Vec::new();
    let mut x = start;
    while x <= hi + step * 1e-9 && v.len() < 20 {
        v.push(x);
        x += step;
    }
    v
}

fn short(x: f64) -> String {
    let a = x.abs();
    if a >= 1e6 {
        format!("{:.1}M", x / 1e6)
    } else if a >= 1e3 {
        format!("{:.1}K", x / 1e3)
    } else if a >= 10.0 || x == x.trunc() {
        format!("{x:.0}")
    } else {
        format!("{x:.2}")
    }
}

pub fn scatter(ui: &mut Ui, t: &Tokens, sc: &Scatter, size: egui::Vec2) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter_at(rect);
    p.rect(rect, 6.0, t.panel, Stroke::new(1.0, t.border), StrokeKind::Inside);
    let plot = Rect::from_min_max(rect.min + vec2(52.0, 14.0), rect.max - vec2(14.0, 40.0));
    let pts: Vec<&(f64, f64, usize)> = sc.points.iter().filter(|p| p.0.is_finite() && p.1.is_finite()).collect();
    if pts.is_empty() {
        p.text(rect.center(), Align2::CENTER_CENTER, "No points", FontId::proportional(12.0), t.muted);
        return;
    }
    let (mut x0, mut x1, mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for q in &pts {
        x0 = x0.min(q.0);
        x1 = x1.max(q.0);
        y0 = y0.min(q.1);
        y1 = y1.max(q.1);
    }
    let pad = |a: f64, b: f64| if b > a { (b - a) * 0.05 } else { 1.0 };
    let (px, py) = (pad(x0, x1), pad(y0, y1));
    let (x0, x1, y0, y1) = (x0 - px, x1 + px, y0 - py, y1 + py);
    let sx = |x: f64| plot.left() + ((x - x0) / (x1 - x0)) as f32 * plot.width();
    let sy = |y: f64| plot.bottom() - ((y - y0) / (y1 - y0)) as f32 * plot.height();
    for tx in nice_ticks(x0, x1, 5) {
        let x = sx(tx);
        p.line_segment([pos2(x, plot.top()), pos2(x, plot.bottom())], Stroke::new(1.0, t.border.gamma_multiply(0.6)));
        p.text(pos2(x, plot.bottom() + 4.0), Align2::CENTER_TOP, short(tx), FontId::proportional(10.0), t.muted);
    }
    for ty in nice_ticks(y0, y1, 4) {
        let y = sy(ty);
        p.line_segment([pos2(plot.left(), y), pos2(plot.right(), y)], Stroke::new(1.0, t.border.gamma_multiply(0.6)));
        p.text(pos2(plot.left() - 6.0, y), Align2::RIGHT_CENTER, short(ty), FontId::proportional(10.0), t.muted);
    }
    p.text(pos2(plot.center().x, rect.bottom() - 8.0), Align2::CENTER_BOTTOM, &sc.x_label, FontId::proportional(11.0), t.text);
    let painter_rot = egui::epaint::TextShape::new(pos2(rect.left() + 6.0, plot.center().y + 30.0), ui.fonts_mut(|f| f.layout_no_wrap(sc.y_label.clone(), FontId::proportional(11.0), t.text)), t.text).with_angle(-std::f32::consts::FRAC_PI_2);
    p.add(painter_rot);
    let r = if pts.len() > 1500 { 2.0 } else if pts.len() > 300 { 3.0 } else { 4.5 };
    for q in &pts {
        let c = if sc.groups.get(q.2).is_some_and(|g| g == "Noise") { Color32::from_gray(150) } else { PALETTE[q.2 % PALETTE.len()] };
        p.circle(pos2(sx(q.0), sy(q.1)), r, c.gamma_multiply(0.8), Stroke::new(0.8, t.panel));
    }
    for (i, c) in sc.centers.iter().enumerate() {
        let pt = pos2(sx(c.0), sy(c.1));
        let col = PALETTE[i % PALETTE.len()];
        p.circle(pt, 8.0, t.panel, Stroke::new(2.5, col));
        p.line_segment([pt - vec2(4.0, 4.0), pt + vec2(4.0, 4.0)], Stroke::new(2.0, col));
        p.line_segment([pt - vec2(4.0, -4.0), pt + vec2(4.0, -4.0)], Stroke::new(2.0, col));
    }
    // Legend.
    if sc.groups.len() > 1 {
        let mut x = plot.left() + 6.0;
        for (i, g) in sc.groups.iter().enumerate() {
            let c = if g == "Noise" { Color32::from_gray(150) } else { PALETTE[i % PALETTE.len()] };
            let w = crate::widgets::text_w(ui, g) + 22.0;
            p.rect(Rect::from_min_size(pos2(x - 4.0, plot.top() + 2.0), vec2(w, 18.0)), 9.0, t.panel.gamma_multiply(0.92), Stroke::new(1.0, t.border), StrokeKind::Inside);
            p.circle_filled(pos2(x + 5.0, plot.top() + 11.0), 4.0, c);
            p.text(pos2(x + 13.0, plot.top() + 11.0), Align2::LEFT_CENTER, g, FontId::proportional(10.5), t.text);
            x += w + 4.0;
        }
    }
    if let Some(h) = resp.hover_pos() {
        let best = pts.iter().map(|q| (q, (pos2(sx(q.0), sy(q.1)) - h).length())).min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        if let Some((q, d)) = best
            && d < 10.0
        {
            resp.on_hover_text(format!("{}: {}\n{}: {}\n{}", sc.x_label, short(q.0), sc.y_label, short(q.1), sc.groups.get(q.2).cloned().unwrap_or_default()));
        }
    }
}

pub fn bars(ui: &mut Ui, t: &Tokens, b: &Bars, width: f32) {
    let n = b.labels.len().min(40);
    if n == 0 {
        return;
    }
    let max = b.values.iter().copied().fold(0.0f64, f64::max).max(1e-12);
    let label_w = b.labels.iter().take(n).map(|l| crate::widgets::text_w(ui, l)).fold(40.0f32, f32::max).min(200.0) + 10.0;
    ui.label(RichText::new(&b.title).size(12.0).strong().color(t.text));
    let bh = 18.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, n as f32 * (bh + 4.0) + 4.0), Sense::hover());
    let p = ui.painter_at(rect);
    for (i, (l, v)) in b.labels.iter().zip(&b.values).take(n).enumerate() {
        let y = rect.top() + 2.0 + i as f32 * (bh + 4.0);
        p.with_clip_rect(Rect::from_min_size(pos2(rect.left(), y), vec2(label_w - 6.0, bh))).text(pos2(rect.left() + label_w - 8.0, y + bh / 2.0), Align2::RIGHT_CENTER, l, FontId::proportional(11.0), t.text);
        let avail = (rect.width() - label_w - 60.0).max(20.0);
        let w = (*v / max) as f32 * avail;
        let col = if b.title.contains("Cluster") { PALETTE[i % PALETTE.len()] } else { t.accent };
        p.rect_filled(Rect::from_min_size(pos2(rect.left() + label_w, y + 2.0), vec2(w.max(1.0), bh - 4.0)), 3.0, col);
        p.text(pos2(rect.left() + label_w + w + 6.0, y + bh / 2.0), Align2::LEFT_CENTER, short(*v), FontId::proportional(10.5), t.muted);
    }
}

pub fn heatmap(ui: &mut Ui, t: &Tokens, rows: &[String], cols: &[String], m: &[Vec<f64>], diverging: bool) {
    let n = rows.len().min(25);
    let k = cols.len().min(25);
    if n == 0 || k == 0 {
        return;
    }
    let label_w = rows.iter().take(n).map(|l| crate::widgets::text_w(ui, l)).fold(30.0f32, f32::max).min(160.0) + 10.0;
    let cell = ((ui.available_width() - label_w - 10.0) / k as f32).clamp(26.0, 64.0);
    let top_h = 22.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(label_w + cell * k as f32, top_h + cell.min(34.0) * n as f32), Sense::hover());
    let ch = cell.min(34.0);
    let p = ui.painter_at(rect);
    let max = m.iter().flatten().copied().filter(|v| v.is_finite()).fold(0.0f64, |a, b| a.max(b.abs())).max(1e-12);
    for (j, c) in cols.iter().take(k).enumerate() {
        let x = rect.left() + label_w + j as f32 * cell;
        p.with_clip_rect(Rect::from_min_size(pos2(x, rect.top()), vec2(cell, top_h))).text(pos2(x + cell / 2.0, rect.top() + top_h / 2.0), Align2::CENTER_CENTER, c, FontId::proportional(10.0), t.muted);
    }
    let mut hover = None;
    for (i, r) in rows.iter().take(n).enumerate() {
        let y = rect.top() + top_h + i as f32 * ch;
        p.text(pos2(rect.left() + label_w - 6.0, y + ch / 2.0), Align2::RIGHT_CENTER, r, FontId::proportional(10.5), t.text);
        for j in 0..k {
            let v = m.get(i).and_then(|row| row.get(j)).copied().unwrap_or(f64::NAN);
            let x = rect.left() + label_w + j as f32 * cell;
            let cr = Rect::from_min_size(pos2(x, y), vec2(cell, ch)).shrink(1.0);
            let col = if !v.is_finite() {
                t.panel_alt
            } else if diverging {
                let a = (v / max).clamp(-1.0, 1.0) as f32;
                if a >= 0.0 { lerp(t.panel, Color32::from_rgb(0x2B, 0x6C, 0xE0), a) } else { lerp(t.panel, Color32::from_rgb(0xE1, 0x4D, 0x4D), -a) }
            } else {
                lerp(t.panel, Color32::from_rgb(0x2B, 0x6C, 0xE0), (v / max) as f32)
            };
            p.rect_filled(cr, 3.0, col);
            let strong = (v.abs() / max) > 0.55;
            let txt = if !v.is_finite() { String::new() } else if diverging { format!("{v:.2}") } else { short(v) };
            p.text(cr.center(), Align2::CENTER_CENTER, txt, FontId::proportional(10.0), if strong { Color32::WHITE } else { t.text });
            if resp.hover_pos().is_some_and(|h| cr.contains(h)) {
                hover = Some(format!("{r} × {}: {}", cols.get(j).cloned().unwrap_or_default(), if v.is_finite() { format!("{v:.4}") } else { "—".into() }));
            }
        }
    }
    if let Some(h) = hover {
        resp.on_hover_text(h);
    }
}

fn lerp(a: Color32, b: Color32, f: f32) -> Color32 {
    let f = f.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * f).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// The summary of a node: metrics, charts, extra tables and the log.
pub fn summary(ui: &mut Ui, t: &Tokens, s: &Summary, id: u64) {
    if !s.metrics.is_empty() {
        metric_cards(ui, t, &s.metrics);
        ui.add_space(8.0);
    }
    let w = ui.available_width();
    if let Some(sc) = &s.scatter {
        scatter(ui, t, sc, vec2(w.min(560.0), 250.0));
        ui.add_space(8.0);
    }
    if let Some((r, c, m)) = &s.heatmap {
        let diverging = m.iter().flatten().any(|v| *v < 0.0) || r.first() == c.first() && m.first().and_then(|x| x.first()).is_some_and(|v| (*v - 1.0).abs() < 1e-9);
        heatmap(ui, t, r, c, m, diverging);
        ui.add_space(8.0);
    }
    if let Some(b) = &s.bars {
        bars(ui, t, b, w.min(520.0));
        ui.add_space(8.0);
    }
    for (i, (name, tb)) in s.tables.iter().enumerate() {
        ui.label(RichText::new(name).size(12.0).strong().color(t.text));
        data_grid(ui, t, tb, ("summary_table", id, i), 220.0);
        ui.add_space(8.0);
    }
    if !s.log.trim().is_empty() {
        ui.label(RichText::new("Output").size(12.0).strong().color(t.text));
        egui::Frame::new().fill(t.panel_alt).stroke(Stroke::new(1.0, t.border)).corner_radius(6.0).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(s.log.trim_end()).monospace().size(11.5).color(t.text));
        });
    }
}
