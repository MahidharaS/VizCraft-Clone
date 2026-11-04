//! Widget windows for the Data Science Studio: data tables, reports, score tables and small plots
//! (scatter, histogram, box, heat maps, curves, bars, dendrogram, forecasts) painted with egui.

use std::collections::BTreeMap;

use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::Value as J;
use vizcraft_core::build::TableData;
use vizcraft_core::stats::Report;
use vizcraft_core::studio::eval::{self, EvalResults, Scores};
use vizcraft_core::studio::exec::View;

use crate::theme::Tokens;

const PALETTE: [Color32; 8] = [
    Color32::from_rgb(0x25, 0x63, 0xEB),
    Color32::from_rgb(0xEA, 0x58, 0x0C),
    Color32::from_rgb(0x16, 0xA3, 0x4A),
    Color32::from_rgb(0xDB, 0x27, 0x77),
    Color32::from_rgb(0x7C, 0x3A, 0xED),
    Color32::from_rgb(0x0D, 0x94, 0x88),
    Color32::from_rgb(0xCA, 0x8A, 0x04),
    Color32::from_rgb(0x64, 0x74, 0x8B),
];

fn color(i: usize) -> Color32 {
    PALETTE[i % PALETTE.len()]
}

fn fmt(v: f64) -> String {
    if !v.is_finite() {
        "–".into()
    } else if v != 0.0 && (v.abs() >= 1e6 || v.abs() < 1e-3) {
        format!("{v:.3e}")
    } else if v.fract() == 0.0 && v.abs() < 1e6 {
        format!("{v:.0}")
    } else {
        format!("{v:.4}")
    }
}

// =============================================================================================
// Plot frame
// =============================================================================================

/// A plotting area with linear x/y scales.
struct Plot {
    rect: Rect,
    x: (f64, f64),
    y: (f64, f64),
}

impl Plot {
    fn px(&self, x: f64, y: f64) -> Pos2 {
        let fx = if self.x.1 > self.x.0 { (x - self.x.0) / (self.x.1 - self.x.0) } else { 0.5 };
        let fy = if self.y.1 > self.y.0 { (y - self.y.0) / (self.y.1 - self.y.0) } else { 0.5 };
        pos2(self.rect.left() + fx as f32 * self.rect.width(), self.rect.bottom() - fy as f32 * self.rect.height())
    }
}

fn range(vals: impl Iterator<Item = f64>) -> (f64, f64) {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for v in vals.filter(|v| v.is_finite()) {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    if !lo.is_finite() {
        return (0.0, 1.0);
    }
    if hi - lo < 1e-12 {
        return (lo - 1.0, hi + 1.0);
    }
    let pad = (hi - lo) * 0.04;
    (lo - pad, hi + pad)
}

/// Allocates a plot with axes, ticks and labels; returns the painter and scales.
fn frame(ui: &mut Ui, t: &Tokens, size: egui::Vec2, x: (f64, f64), y: (f64, f64), xl: &str, yl: &str) -> (egui::Painter, Plot) {
    frame_ex(ui, t, size, x, y, xl, yl, true)
}

/// `frame` without numeric x ticks (for categorical x axes).
#[allow(clippy::too_many_arguments)]
fn frame_ex(ui: &mut Ui, t: &Tokens, size: egui::Vec2, x: (f64, f64), y: (f64, f64), xl: &str, yl: &str, xticks: bool) -> (egui::Painter, Plot) {
    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter_at(r);
    let area = Rect::from_min_max(r.min + vec2(52.0, 20.0), r.max - vec2(10.0, 30.0));
    p.rect_filled(area, 0.0, t.panel);
    p.rect_stroke(area, 0.0, Stroke::new(1.0, t.border), StrokeKind::Inside);
    let plot = Plot { rect: area, x, y };
    for i in 0..=4 {
        let f = i as f64 / 4.0;
        let yv = y.0 + (y.1 - y.0) * f;
        let py = plot.px(x.0, yv).y;
        p.line_segment([pos2(area.left(), py), pos2(area.right(), py)], Stroke::new(0.5, t.border));
        p.text(pos2(area.left() - 4.0, py), Align2::RIGHT_CENTER, short(yv), FontId::proportional(9.5), t.muted);
        if xticks {
            let xv = x.0 + (x.1 - x.0) * f;
            let px = plot.px(xv, y.0).x;
            p.text(pos2(px, area.bottom() + 3.0), Align2::CENTER_TOP, short(xv), FontId::proportional(9.5), t.muted);
        }
    }
    p.text(pos2(area.center().x, r.bottom() - 2.0), Align2::CENTER_BOTTOM, xl, FontId::proportional(10.5), t.text);
    p.text(pos2(area.left(), r.top() + 2.0), Align2::LEFT_TOP, yl, FontId::proportional(10.5), t.text);
    (p, plot)
}

fn short(v: f64) -> String {
    let a = v.abs();
    if a >= 1e6 {
        format!("{:.1}M", v / 1e6)
    } else if a >= 1e3 {
        format!("{:.1}K", v / 1e3)
    } else if a >= 10.0 || v == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.2}")
    }
}

fn size(ui: &Ui, h: f32) -> egui::Vec2 {
    vec2((ui.available_width() - 10.0).clamp(300.0, 900.0), h)
}

fn polyline(p: &egui::Painter, pl: &Plot, pts: &[(f64, f64)], c: Color32, w: f32) {
    let v: Vec<Pos2> = pts.iter().filter(|(x, y)| x.is_finite() && y.is_finite()).map(|(x, y)| pl.px(*x, *y)).collect();
    if v.len() > 1 {
        p.add(egui::Shape::line(v, Stroke::new(w, c)));
    }
}

fn legend(ui: &mut Ui, items: &[(String, Color32)]) {
    ui.horizontal_wrapped(|ui| {
        for (l, c) in items {
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().rect_filled(r, 2.0, *c);
            ui.label(RichText::new(l).size(11.0));
            ui.add_space(6.0);
        }
    });
}

/// A colour scale from the panel colour to the accent (0..1), or diverging (-1..1).
fn heat(t: &Tokens, v: f64, diverging: bool) -> Color32 {
    let (lo, hi) = (Color32::from_rgb(0xDC, 0x26, 0x26), Color32::from_rgb(0x25, 0x63, 0xEB));
    let mix = |a: Color32, b: Color32, f: f32| Color32::from_rgb((a.r() as f32 + (b.r() as f32 - a.r() as f32) * f) as u8, (a.g() as f32 + (b.g() as f32 - a.g() as f32) * f) as u8, (a.b() as f32 + (b.b() as f32 - a.b() as f32) * f) as u8);
    let base = if t.dark { Color32::from_rgb(0x1F, 0x24, 0x2D) } else { Color32::WHITE };
    if !v.is_finite() {
        return base;
    }
    if diverging {
        if v >= 0.0 { mix(base, hi, v.min(1.0) as f32) } else { mix(base, lo, (-v).min(1.0) as f32) }
    } else {
        mix(base, hi, v.clamp(0.0, 1.0) as f32)
    }
}

/// A grid of cells with row/column headers, coloured by `values` (already scaled).
fn matrix(ui: &mut Ui, t: &Tokens, rows: &[String], cols: &[String], text: &[Vec<String>], values: &[Vec<f64>], diverging: bool) {
    let cw = ((ui.available_width() - 140.0) / cols.len().max(1) as f32).clamp(36.0, 110.0);
    let ch = 24.0;
    let (r, _) = ui.allocate_exact_size(vec2(130.0 + cw * cols.len() as f32, ch * (rows.len() + 1) as f32), Sense::hover());
    let p = ui.painter_at(r);
    for (j, c) in cols.iter().enumerate() {
        let cr = Rect::from_min_size(r.min + vec2(130.0 + cw * j as f32, 0.0), vec2(cw, ch));
        p.text(cr.center(), Align2::CENTER_CENTER, trunc(c, 14), FontId::proportional(10.0), t.muted);
    }
    for (i, rl) in rows.iter().enumerate() {
        let y = ch * (i + 1) as f32;
        p.text(r.min + vec2(126.0, y + ch / 2.0), Align2::RIGHT_CENTER, trunc(rl, 20), FontId::proportional(10.5), t.text);
        for j in 0..cols.len() {
            let cr = Rect::from_min_size(r.min + vec2(130.0 + cw * j as f32, y), vec2(cw, ch)).shrink(1.0);
            let v = values.get(i).and_then(|r| r.get(j)).copied().unwrap_or(f64::NAN);
            let fill = heat(t, v, diverging);
            p.rect_filled(cr, 2.0, fill);
            let txt_c = if v.abs() > 0.6 { Color32::WHITE } else { t.text };
            p.text(cr.center(), Align2::CENTER_CENTER, text.get(i).and_then(|r| r.get(j)).cloned().unwrap_or_default(), FontId::proportional(10.0), txt_c);
        }
    }
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() > n { format!("{}…", s.chars().take(n - 1).collect::<String>()) } else { s.to_string() }
}

// =============================================================================================
// Data, reports, scores
// =============================================================================================

pub fn data_table(ui: &mut Ui, t: &Tokens, d: &TableData, target: Option<&str>) {
    ui.label(RichText::new(format!("{} rows × {} columns{}", d.rows.len(), d.columns.len(), target.map(|x| format!(" · target: {x}")).unwrap_or_default())).size(11.0).color(t.muted));
    egui::Grid::new(("st-data", d.name.as_str(), d.columns.len())).striped(true).spacing([14.0, 3.0]).show(ui, |ui| {
        ui.label(RichText::new("#").size(10.5).color(t.muted));
        for (i, c) in d.columns.iter().enumerate() {
            let ty = d.types.get(i).map(|x| if x.is_temporal() { "date" } else if x.is_numeric() { "num" } else { "cat" }).unwrap_or("");
            let is_target = target == Some(c.as_str());
            ui.vertical(|ui| {
                ui.label(RichText::new(c).strong().size(11.0).color(if is_target { t.accent } else { t.text }));
                ui.label(RichText::new(ty).size(9.0).color(t.muted));
            });
        }
        ui.end_row();
        for (ri, r) in d.rows.iter().take(300).enumerate() {
            ui.label(RichText::new((ri + 1).to_string()).size(10.0).color(t.muted));
            for (ci, v) in r.iter().enumerate() {
                let temporal = d.types.get(ci).is_some_and(|x| x.is_temporal());
                let s = match v.as_f64() {
                    Some(x) if temporal && !matches!(v, vizcraft_core::scalar::Scalar::Text(_)) => vizcraft_core::numfmt::format_date(x, if x.fract() == 0.0 { "yyyy-mm-dd" } else { "yyyy-mm-dd hh:mm" }),
                    Some(x) if !matches!(v, vizcraft_core::scalar::Scalar::Text(_)) => fmt(x),
                    _ => v.text(),
                };
                ui.label(RichText::new(trunc(&s, 28)).size(11.0));
            }
            ui.end_row();
        }
    });
    if d.rows.len() > 300 {
        ui.label(RichText::new(format!("showing 300 of {} rows", d.rows.len())).size(10.5).color(t.muted));
    }
}

pub fn report(ui: &mut Ui, t: &Tokens, r: &Report) {
    if !r.title.is_empty() {
        ui.label(RichText::new(&r.title).strong().size(13.0));
    }
    for (si, s) in r.sections.iter().enumerate() {
        ui.add_space(6.0);
        if !s.heading.is_empty() {
            ui.label(RichText::new(&s.heading).strong().size(12.0).color(t.accent));
        }
        if !s.kv.is_empty() {
            egui::Grid::new(("st-kv", si, s.heading.as_str())).spacing([18.0, 2.0]).show(ui, |ui| {
                for (k, v) in &s.kv {
                    ui.label(RichText::new(k).size(11.0).color(t.muted));
                    ui.label(RichText::new(v).size(11.0));
                    ui.end_row();
                }
            });
        }
        if let Some(tb) = &s.table {
            egui::Grid::new(("st-rt", si, s.heading.as_str())).striped(true).spacing([16.0, 2.0]).show(ui, |ui| {
                for c in &tb.columns {
                    ui.label(RichText::new(c).strong().size(11.0));
                }
                ui.end_row();
                for row in &tb.rows {
                    for (j, c) in row.iter().enumerate() {
                        let sig = tb.p_col == Some(j) && c.trim_start_matches('<').parse::<f64>().is_ok_and(|p| p < 0.05);
                        ui.label(RichText::new(c).size(11.0).color(if sig { t.good } else { t.text }).strong_if(sig));
                    }
                    ui.end_row();
                }
            });
        }
        if !s.note.is_empty() {
            ui.label(RichText::new(&s.note).size(11.0).italics());
        }
    }
}

trait StrongIf {
    fn strong_if(self, b: bool) -> Self;
}
impl StrongIf for RichText {
    fn strong_if(self, b: bool) -> Self {
        if b { self.strong() } else { self }
    }
}

pub fn scores(ui: &mut Ui, t: &Tokens, s: &[Scores], classification: bool) {
    let Some(first) = s.first() else {
        ui.label(RichText::new("No scores").color(t.muted));
        return;
    };
    ui.label(RichText::new(if classification { "Classification scores" } else { "Regression scores" }).strong().size(12.0));
    let names: Vec<String> = first.values.iter().map(|(n, _)| n.clone()).collect();
    let best: Vec<f64> = names
        .iter()
        .map(|n| {
            let lower_better = matches!(n.as_str(), "MSE" | "RMSE" | "MAE" | "LogLoss" | "Log loss" | "MAPE");
            let vals = s.iter().map(|x| x.get(n)).filter(|v| v.is_finite());
            if lower_better { vals.fold(f64::INFINITY, f64::min) } else { vals.fold(f64::NEG_INFINITY, f64::max) }
        })
        .collect();
    egui::Grid::new("st-scores").striped(true).spacing([18.0, 3.0]).show(ui, |ui| {
        ui.label(RichText::new("Model").strong().size(11.0));
        for n in &names {
            ui.label(RichText::new(n).strong().size(11.0));
        }
        ui.end_row();
        for sc in s {
            ui.label(RichText::new(&sc.learner).size(11.0));
            for (i, n) in names.iter().enumerate() {
                let v = sc.get(n);
                let b = s.len() > 1 && (v - best[i]).abs() < 1e-12;
                let txt = if v.is_finite() { format!("{v:.4}") } else { "–".into() };
                ui.label(RichText::new(txt).size(11.0).color(if b { t.good } else { t.text }).strong_if(b));
            }
            ui.end_row();
        }
    });
}

fn bars(ui: &mut Ui, t: &Tokens, items: &[(String, f64)], title: &str, signed: bool) {
    if !title.is_empty() {
        ui.label(RichText::new(title).strong().size(12.0));
    }
    let maxv = items.iter().map(|(_, v)| v.abs()).filter(|v| v.is_finite()).fold(0.0, f64::max).max(1e-12);
    let w = (ui.available_width() - 220.0).clamp(160.0, 600.0);
    for (i, (l, v)) in items.iter().enumerate() {
        ui.horizontal(|ui| {
            let (lr, _) = ui.allocate_exact_size(vec2(170.0, 16.0), Sense::hover());
            ui.painter().text(lr.left_center(), Align2::LEFT_CENTER, trunc(l, 28), FontId::proportional(11.0), t.text);
            let (r, _) = ui.allocate_exact_size(vec2(w, 14.0), Sense::hover());
            let p = ui.painter();
            let f = (v.abs() / maxv) as f32;
            if signed {
                let mid = r.center().x;
                let bw = f * r.width() / 2.0;
                let br = if *v >= 0.0 { Rect::from_min_size(pos2(mid, r.top()), vec2(bw, r.height())) } else { Rect::from_min_size(pos2(mid - bw, r.top()), vec2(bw, r.height())) };
                p.line_segment([pos2(mid, r.top()), pos2(mid, r.bottom())], Stroke::new(1.0, t.border));
                p.rect_filled(br, 2.0, if *v >= 0.0 { color(0) } else { color(1) });
            } else {
                p.rect_filled(Rect::from_min_size(r.min, vec2(f * r.width(), r.height())), 2.0, color(i.min(0)));
            }
            ui.label(RichText::new(fmt(*v)).size(10.5).color(t.muted));
        });
    }
}

fn series_plot(ui: &mut Ui, t: &Tokens, series: &[(&str, Vec<(f64, f64)>, Color32)], xl: &str, yl: &str, h: f32) {
    let xr = range(series.iter().flat_map(|(_, s, _)| s.iter().map(|p| p.0)));
    let yr = range(series.iter().flat_map(|(_, s, _)| s.iter().map(|p| p.1)));
    legend(ui, &series.iter().map(|(n, _, c)| (n.to_string(), *c)).collect::<Vec<_>>());
    let (p, pl) = frame(ui, t, size(ui, h), xr, yr, xl, yl);
    for (_, s, c) in series {
        polyline(&p, &pl, s, *c, 1.6);
    }
}

fn idx(v: &[f64]) -> Vec<(f64, f64)> {
    v.iter().enumerate().map(|(i, y)| (i as f64, *y)).collect()
}

/// Windows for views computed by the executor.
pub fn view(ui: &mut Ui, t: &Tokens, v: &View) {
    match v {
        View::None => {}
        View::Report(r) => report(ui, t, r),
        View::Text(s) => {
            ui.label(RichText::new(s).monospace().size(11.0));
        }
        View::Regression { report: r, fitted, resid } => {
            ui.columns(2, |c| {
                report(&mut c[0], t, r);
                c[1].label(RichText::new("Residuals vs fitted").strong().size(12.0));
                let pts: Vec<(f64, f64)> = fitted.iter().zip(resid).map(|(a, b)| (*a, *b)).collect();
                let w1 = c[1].available_width().max(260.0);
                let (p, pl) = frame(&mut c[1], t, vec2(w1, 260.0), range(pts.iter().map(|p| p.0)), range(pts.iter().map(|p| p.1)), "fitted", "residual");
                let z = pl.px(pl.x.0, 0.0).y;
                p.line_segment([pos2(pl.rect.left(), z), pos2(pl.rect.right(), z)], Stroke::new(1.0, t.muted));
                for (x, y) in pts.iter().take(5000) {
                    p.circle_filled(pl.px(*x, *y), 2.0, color(0).gamma_multiply(0.6));
                }
            });
        }
        View::Normality { report: r, values } => {
            ui.columns(2, |c| {
                report(&mut c[0], t, r);
                c[1].label(RichText::new("Normal Q–Q plot").strong().size(12.0));
                let mut s: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
                s.sort_by(|a, b| a.total_cmp(b));
                let n = s.len().max(1) as f64;
                let m = s.iter().sum::<f64>() / n;
                let sd = (s.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (n - 1.0).max(1.0)).sqrt().max(1e-12);
                let pts: Vec<(f64, f64)> = s.iter().enumerate().map(|(i, v)| (vizcraft_core::stats::special::norm_ppf((i as f64 + 0.5) / n), (v - m) / sd)).collect();
                let r = range(pts.iter().flat_map(|p| [p.0, p.1]));
                let w1 = c[1].available_width().max(260.0);
                let (p, pl) = frame(&mut c[1], t, vec2(w1, 260.0), r, r, "theoretical", "sample (standardised)");
                polyline(&p, &pl, &[(r.0, r.0), (r.1, r.1)], t.muted, 1.0);
                for (x, y) in pts.iter().take(5000) {
                    p.circle_filled(pl.px(*x, *y), 2.0, color(0));
                }
            });
        }
        View::Forecast { report: r, history, fitted, forecast, labels: _ } => {
            let n = history.len();
            let fc: Vec<(f64, f64)> = forecast.mean.iter().enumerate().map(|(i, v)| ((n + i) as f64, *v)).collect();
            let lo: Vec<(f64, f64)> = forecast.lower.iter().enumerate().map(|(i, v)| ((n + i) as f64, *v)).collect();
            let hi: Vec<(f64, f64)> = forecast.upper.iter().enumerate().map(|(i, v)| ((n + i) as f64, *v)).collect();
            series_plot(ui, t, &[("history", idx(history), color(0)), ("fitted", idx(fitted), color(7)), ("forecast", fc, color(1)), ("lower", lo, color(1).gamma_multiply(0.4)), ("upper", hi, color(1).gamma_multiply(0.4))], "period", "value", 260.0);
            report(ui, t, r);
        }
        View::Decomposition(d) => {
            for (name, s, c) in [("observed", &d.observed, 0), ("trend", &d.trend, 1), ("seasonal", &d.seasonal, 2), ("residual", &d.resid, 3)] {
                series_plot(ui, t, &[(name, idx(s), color(c))], "", "", 120.0);
            }
        }
        View::Acf { acf, pacf, band, n } => {
            for (name, s) in [("ACF", acf), ("PACF", pacf)] {
                ui.label(RichText::new(format!("{name} (n = {n}, band ±{band:.3})")).strong().size(12.0));
                let (p, pl) = frame(ui, t, size(ui, 150.0), (-0.5, s.len() as f64 - 0.5), (-1.0, 1.0), "lag", name);
                for b in [*band, -*band] {
                    let y = pl.px(0.0, b).y;
                    p.line_segment([pos2(pl.rect.left(), y), pos2(pl.rect.right(), y)], Stroke::new(1.0, t.danger.gamma_multiply(0.6)));
                }
                for (i, v) in s.iter().enumerate() {
                    p.line_segment([pl.px(i as f64, 0.0), pl.px(i as f64, *v)], Stroke::new(3.0, color(0)));
                }
            }
        }
        View::Distribution { report: r, values, fits } => {
            hist(ui, t, &[("values".into(), values.clone())], 30, "value");
            ui.label(RichText::new("Fitted distributions").strong().size(12.0));
            egui::Grid::new("st-fits").striped(true).spacing([16.0, 2.0]).show(ui, |ui| {
                for h in ["Distribution", "AIC", "BIC", "KS", "KS p"] {
                    ui.label(RichText::new(h).strong().size(11.0));
                }
                ui.end_row();
                for f in fits {
                    ui.label(RichText::new(f.dist.name()).size(11.0));
                    for v in [f.aic, f.bic, f.ks, f.ks_p] {
                        ui.label(RichText::new(fmt(v)).size(11.0));
                    }
                    ui.end_row();
                }
            });
            report(ui, t, r);
        }
        View::Scores { scores: s, method, errors, classification } => {
            ui.label(RichText::new(method).size(11.0).color(t.muted));
            scores(ui, t, s, *classification);
            for (l, e) in errors {
                ui.label(RichText::new(format!("{l}: {e}")).size(11.0).color(t.danger));
            }
        }
        View::Rank(rows) => {
            let names: Vec<String> = rows.first().map(|r| r.scores.iter().map(|(n, _)| n.clone()).collect()).unwrap_or_default();
            egui::Grid::new("st-rank").striped(true).spacing([16.0, 2.0]).show(ui, |ui| {
                ui.label(RichText::new("Feature").strong().size(11.0));
                for n in &names {
                    ui.label(RichText::new(n).strong().size(11.0));
                }
                ui.end_row();
                for r in rows {
                    ui.label(RichText::new(format!("{} {}", if r.categorical { "▤" } else { "#" }, r.feature)).size(11.0));
                    for (_, v) in &r.scores {
                        ui.label(RichText::new(fmt(*v)).size(11.0));
                    }
                    ui.end_row();
                }
            });
        }
        View::Importance(items) => {
            let it: Vec<(String, f64)> = items.iter().map(|(n, m, _)| (n.clone(), *m)).collect();
            bars(ui, t, &it, "Permutation importance (drop in score when the feature is shuffled)", false);
        }
        View::Explain { contributions, prediction } => {
            ui.label(RichText::new(format!("Prediction: {prediction}")).strong().size(12.5));
            bars(ui, t, contributions, "Contribution of each feature", true);
        }
        View::Dendrogram { merges, labels, clusters, k } => dendrogram(ui, t, merges, labels, clusters, *k),
        View::Silhouette { values, clusters, names } => {
            let mut idx: Vec<usize> = (0..values.len()).collect();
            idx.sort_by(|a, b| clusters[*a].cmp(&clusters[*b]).then(values[*b].total_cmp(&values[*a])));
            let avg = values.iter().sum::<f64>() / values.len().max(1) as f64;
            ui.label(RichText::new(format!("Average silhouette {avg:.3}")).strong().size(12.0));
            let h = (idx.len() as f32 * 1.2).clamp(160.0, 420.0);
            let (p, pl) = frame(ui, t, size(ui, h), (-1.0, 1.0), (0.0, idx.len() as f64), "silhouette", "");
            for (row, &i) in idx.iter().enumerate() {
                let y = pl.px(0.0, row as f64 + 0.5).y;
                p.line_segment([pos2(pl.px(0.0, 0.0).x, y), pos2(pl.px(values[i], 0.0).x, y)], Stroke::new((h / idx.len().max(1) as f32).max(1.0), color(clusters[i])));
            }
            legend(ui, &names.iter().enumerate().map(|(i, n)| (n.clone(), color(i))).collect::<Vec<_>>());
        }
        View::Clusters { sizes, silhouette, curve } => {
            ui.label(RichText::new(format!("{} clusters · silhouette {silhouette:.3}", sizes.len())).strong().size(12.0));
            bars(ui, t, &sizes.iter().enumerate().map(|(i, s)| (format!("C{}", i + 1), *s as f64)).collect::<Vec<_>>(), "Cluster sizes", false);
            if curve.len() > 1 {
                series_plot(ui, t, &[("silhouette by k", curve.iter().map(|(k, s)| (*k as f64, *s)).collect(), color(0))], "k", "silhouette", 180.0);
            }
        }
        View::Pca { explained, loadings, names } => {
            let mut cum = 0.0;
            let pts: Vec<(f64, f64)> = explained.iter().enumerate().map(|(i, e)| (i as f64 + 1.0, *e)).collect();
            let cpts: Vec<(f64, f64)> = explained.iter().enumerate().map(|(i, e)| {
                cum += e;
                (i as f64 + 1.0, cum)
            }).collect();
            series_plot(ui, t, &[("explained variance", pts, color(0)), ("cumulative", cpts, color(1))], "component", "proportion", 200.0);
            let comps = loadings.len().min(4);
            if comps > 0 {
                let cols: Vec<String> = (1..=comps).map(|i| format!("PC{i}")).collect();
                let text: Vec<Vec<String>> = names.iter().enumerate().map(|(f, _)| (0..comps).map(|c| format!("{:.2}", loadings[c].get(f).copied().unwrap_or(f64::NAN))).collect()).collect();
                let vals: Vec<Vec<f64>> = names.iter().enumerate().map(|(f, _)| (0..comps).map(|c| loadings[c].get(f).copied().unwrap_or(f64::NAN)).collect()).collect();
                ui.label(RichText::new("Loadings").strong().size(12.0));
                matrix(ui, t, names, &cols, &text, &vals, true);
            }
        }
        View::Model { summary, coefficients, classes } => {
            ui.label(RichText::new(summary).size(11.5));
            if !coefficients.is_empty() {
                let cols: Vec<String> = if classes.len() > 2 { classes.clone() } else { vec!["coefficient".into()] };
                egui::Grid::new("st-coef").striped(true).spacing([16.0, 2.0]).show(ui, |ui| {
                    ui.label(RichText::new("Term").strong().size(11.0));
                    for c in &cols {
                        ui.label(RichText::new(c).strong().size(11.0));
                    }
                    ui.end_row();
                    for (n, v) in coefficients {
                        ui.label(RichText::new(n).size(11.0));
                        for x in v {
                            ui.label(RichText::new(fmt(*x)).size(11.0));
                        }
                        ui.end_row();
                    }
                });
            }
        }
        View::Drift(rows) => {
            egui::Grid::new("st-drift").striped(true).spacing([16.0, 2.0]).show(ui, |ui| {
                for h in ["Feature", "PSI", "KS", "KS p", "Train mean", "New mean", "Missing train", "Missing new", ""] {
                    ui.label(RichText::new(h).strong().size(11.0));
                }
                ui.end_row();
                for r in rows {
                    ui.label(RichText::new(&r.feature).size(11.0));
                    for v in [r.psi, r.ks, r.ks_p, r.train_mean, r.new_mean, r.missing_train, r.missing_new] {
                        ui.label(RichText::new(fmt(v)).size(11.0));
                    }
                    let (l, c) = if r.psi >= 0.25 { ("major drift", t.danger) } else if r.psi >= 0.1 { ("some drift", Color32::from_rgb(0xD9, 0x77, 0x06)) } else { ("stable", t.good) };
                    ui.label(RichText::new(l).size(11.0).color(c));
                    ui.end_row();
                }
            });
        }
    }
}

fn dendrogram(ui: &mut Ui, t: &Tokens, merges: &[vizcraft_core::studio::unsup::Merge], labels: &[String], clusters: &[usize], k: usize) {
    let n = labels.len();
    ui.label(RichText::new(format!("{n} items · cut into {k} clusters")).strong().size(12.0));
    if n == 0 {
        return;
    }
    // Leaf order from the merge tree (depth-first), node x positions, node heights.
    let total = n + merges.len();
    let mut children: Vec<Option<(usize, usize)>> = vec![None; total];
    let mut height = vec![0.0; total];
    for (i, m) in merges.iter().enumerate() {
        children[n + i] = Some((m.a, m.b));
        height[n + i] = m.height;
    }
    let mut order = vec![];
    let root = total - 1;
    let mut stack = vec![root];
    while let Some(x) = stack.pop() {
        match children.get(x).copied().flatten() {
            Some((a, b)) => {
                stack.push(b);
                stack.push(a);
            }
            None => order.push(x),
        }
    }
    let mut xpos = vec![0.0; total];
    for (i, leaf) in order.iter().enumerate() {
        if *leaf < total {
            xpos[*leaf] = i as f64;
        }
    }
    for i in 0..merges.len() {
        let (a, b) = (merges[i].a, merges[i].b);
        xpos[n + i] = (xpos.get(a).copied().unwrap_or(0.0) + xpos.get(b).copied().unwrap_or(0.0)) / 2.0;
    }
    let hmax = merges.iter().map(|m| m.height).fold(0.0, f64::max).max(1e-12);
    let (p, pl) = frame(ui, t, size(ui, 300.0), (-0.5, n as f64 - 0.5), (0.0, hmax * 1.05), "", "distance");
    for i in 0..merges.len() {
        let (a, b) = (merges[i].a, merges[i].b);
        let h = height[n + i];
        let c = if a < n { color(clusters.get(a).copied().unwrap_or(0)) } else { t.muted };
        let pts = [pl.px(xpos[a], height[a]), pl.px(xpos[a], h), pl.px(xpos[b], h), pl.px(xpos[b], height[b])];
        p.add(egui::Shape::line(pts.to_vec(), Stroke::new(1.0, c)));
    }
    if n <= 60 {
        for (i, leaf) in order.iter().enumerate() {
            if let Some(l) = labels.get(*leaf) {
                p.text(pl.px(i as f64, 0.0) + vec2(0.0, 2.0), Align2::CENTER_TOP, trunc(l, 6), FontId::proportional(8.0), color(clusters.get(*leaf).copied().unwrap_or(0)));
            }
        }
    }
}

// =============================================================================================
// Evaluation windows
// =============================================================================================

pub fn confusion(ui: &mut Ui, t: &Tokens, r: &EvalResults, learner: usize, show: &str) {
    if !r.is_classification() {
        ui.label(RichText::new("The confusion matrix needs a classification task.").color(t.muted));
        return;
    }
    let m = eval::confusion(r, learner.min(r.learners.len().saturating_sub(1)));
    ui.label(RichText::new(format!("{} — rows: actual, columns: predicted", r.learners.get(learner).cloned().unwrap_or_default())).strong().size(12.0));
    let total: usize = m.iter().flatten().sum();
    let rows_sum: Vec<usize> = m.iter().map(|r| r.iter().sum()).collect();
    let cols_sum: Vec<usize> = (0..r.classes.len()).map(|j| m.iter().map(|r| r.get(j).copied().unwrap_or(0)).sum()).collect();
    let mut text = vec![];
    let mut vals = vec![];
    for (i, row) in m.iter().enumerate() {
        let mut tr = vec![];
        let mut vr = vec![];
        for (j, c) in row.iter().enumerate() {
            let v = match show {
                "row" => *c as f64 / rows_sum[i].max(1) as f64,
                "col" => *c as f64 / cols_sum[j].max(1) as f64,
                _ => *c as f64,
            };
            tr.push(if show == "row" || show == "col" { format!("{:.1}%", v * 100.0) } else { c.to_string() });
            vr.push(*c as f64 / rows_sum[i].max(1) as f64 * if i == j { 1.0 } else { 0.5 });
        }
        text.push(tr);
        vals.push(vr);
    }
    matrix(ui, t, &r.classes, &r.classes, &text, &vals, false);
    let correct: usize = (0..m.len()).map(|i| m[i].get(i).copied().unwrap_or(0)).sum();
    ui.label(RichText::new(format!("Accuracy {:.1}% ({correct} of {total})", correct as f64 / total.max(1) as f64 * 100.0)).size(11.5));
}

pub fn curves(ui: &mut Ui, t: &Tokens, r: &EvalResults, kind: &str, class: usize, curve: &str, bins: usize) {
    if !r.is_classification() {
        ui.label(RichText::new("This needs a classification task.").color(t.muted));
        return;
    }
    let class = class.min(r.classes.len().saturating_sub(1));
    let cls = r.classes.get(class).cloned().unwrap_or_default();
    let mut series: Vec<(String, Vec<(f64, f64)>, Color32)> = vec![];
    for (l, name) in r.learners.iter().enumerate() {
        let (pts, label) = match kind {
            "eval.roc" => {
                let c = eval::roc(r, l, class);
                (c.points.iter().map(|p| (p.0, p.1)).collect::<Vec<_>>(), format!("{name} (AUC {:.3})", c.auc))
            }
            "eval.lift" => {
                let c = eval::gains(r, l, class);
                let pts: Vec<(f64, f64)> = if curve == "gain" { c.points.iter().map(|p| (p.0, p.1)).collect() } else { c.points.iter().filter(|p| p.0 > 0.01).map(|p| (p.0, p.1 / p.0)).collect() };
                (pts, name.clone())
            }
            _ => {
                let c = eval::calibration(r, l, class, bins.max(3));
                (c.iter().map(|(a, b, _)| (*a, *b)).collect(), format!("{name} (Brier {:.3})", eval::brier(r, l, class)))
            }
        };
        series.push((label, pts, color(l)));
    }
    let (title, xl, yl) = match kind {
        "eval.roc" => ("ROC curve", "false positive rate", "true positive rate"),
        "eval.lift" if curve == "gain" => ("Cumulative gains", "proportion of instances", "proportion of positives"),
        "eval.lift" => ("Lift", "proportion of instances", "lift"),
        _ => ("Calibration", "predicted probability", "observed frequency"),
    };
    ui.label(RichText::new(format!("{title} · target class {cls}")).strong().size(12.0));
    legend(ui, &series.iter().map(|(n, _, c)| (n.clone(), *c)).collect::<Vec<_>>());
    let yr = if kind == "eval.lift" && curve != "gain" { range(series.iter().flat_map(|s| s.1.iter().map(|p| p.1)).chain([0.0, 1.0])) } else { (0.0, 1.0) };
    let (p, pl) = frame(ui, t, size(ui, 300.0), (0.0, 1.0), yr, xl, yl);
    if kind != "eval.lift" || curve == "gain" {
        polyline(&p, &pl, &[(0.0, 0.0), (1.0, 1.0)], t.muted, 1.0);
    } else {
        polyline(&p, &pl, &[(0.0, 1.0), (1.0, 1.0)], t.muted, 1.0);
    }
    for (_, pts, c) in &series {
        polyline(&p, &pl, pts, *c, 2.0);
        if kind == "eval.calibration" {
            for (x, y) in pts {
                p.circle_filled(pl.px(*x, *y), 3.0, *c);
            }
        }
    }
}

// =============================================================================================
// Visualize widgets (computed from the input data)
// =============================================================================================

fn setting(s: &BTreeMap<String, J>, k: &str) -> String {
    s.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn num_col(d: &TableData, c: &str) -> Option<Vec<f64>> {
    let i = d.column_index(c)?;
    Some(d.rows.iter().map(|r| r.get(i).and_then(|v| v.as_f64()).unwrap_or(f64::NAN)).collect())
}

fn cat_col(d: &TableData, c: &str) -> Option<Vec<String>> {
    let i = d.column_index(c)?;
    Some(d.rows.iter().map(|r| r.get(i).map(|v| v.text()).unwrap_or_default()).collect())
}

fn numeric_columns(d: &TableData, chosen: &[String]) -> Vec<String> {
    if !chosen.is_empty() {
        return chosen.iter().filter(|c| d.column_index(c).is_some()).cloned().collect();
    }
    d.columns.iter().enumerate().filter(|(i, _)| d.types.get(*i).is_some_and(|t| t.is_numeric())).map(|(_, c)| c.clone()).collect()
}

fn levels(v: &[String]) -> Vec<String> {
    let mut l: Vec<String> = v.iter().cloned().collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    l.truncate(24);
    l
}

fn pearson(x: &[f64], y: &[f64]) -> f64 {
    let pts: Vec<(f64, f64)> = x.iter().zip(y).filter(|(a, b)| a.is_finite() && b.is_finite()).map(|(a, b)| (*a, *b)).collect();
    let n = pts.len() as f64;
    if n < 3.0 {
        return f64::NAN;
    }
    let (mx, my) = (pts.iter().map(|p| p.0).sum::<f64>() / n, pts.iter().map(|p| p.1).sum::<f64>() / n);
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (a, b) in &pts {
        sxy += (a - mx) * (b - my);
        sxx += (a - mx).powi(2);
        syy += (b - my).powi(2);
    }
    sxy / (sxx * syy).sqrt().max(1e-300)
}

fn ranks(x: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..x.len()).collect();
    idx.sort_by(|a, b| x[*a].total_cmp(&x[*b]));
    let mut r = vec![f64::NAN; x.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i;
        while j + 1 < idx.len() && x[idx[j + 1]] == x[idx[i]] {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0 + 1.0;
        for k in i..=j {
            r[idx[k]] = if x[idx[k]].is_finite() { avg } else { f64::NAN };
        }
        i = j + 1;
    }
    r
}

fn hist(ui: &mut Ui, t: &Tokens, groups: &[(String, Vec<f64>)], bins: usize, xl: &str) {
    let all: Vec<f64> = groups.iter().flat_map(|g| g.1.iter().copied()).filter(|v| v.is_finite()).collect();
    if all.is_empty() {
        ui.label(RichText::new("No numeric values").color(t.muted));
        return;
    }
    let (lo, hi) = range(all.iter().copied());
    let bins = bins.clamp(2, 100);
    let w = (hi - lo) / bins as f64;
    let counts: Vec<Vec<f64>> = groups
        .iter()
        .map(|(_, v)| {
            let mut c = vec![0.0; bins];
            for x in v.iter().filter(|x| x.is_finite()) {
                c[(((x - lo) / w) as usize).min(bins - 1)] += 1.0;
            }
            c
        })
        .collect();
    let ymax = (0..bins).map(|b| counts.iter().map(|c| c[b]).sum::<f64>()).fold(0.0, f64::max);
    if groups.len() > 1 {
        legend(ui, &groups.iter().enumerate().map(|(i, g)| (g.0.clone(), color(i))).collect::<Vec<_>>());
    }
    let (p, pl) = frame(ui, t, size(ui, 260.0), (lo, hi), (0.0, ymax * 1.05), xl, "count");
    for b in 0..bins {
        let mut base = 0.0;
        for (g, c) in counts.iter().enumerate() {
            let a = pl.px(lo + w * b as f64, base);
            let z = pl.px(lo + w * (b + 1) as f64, base + c[b]);
            p.rect_filled(Rect::from_two_pos(a, z).shrink2(vec2(0.5, 0.0)), 0.0, color(g).gamma_multiply(0.85));
            base += c[b];
        }
    }
}

fn crosstab(ui: &mut Ui, t: &Tokens, d: &TableData, xc: &str, yc: &str, mosaic: bool) {
    let (Some(x), Some(y)) = (cat_col(d, xc), cat_col(d, yc)) else {
        ui.label(RichText::new("Choose two categorical columns").color(t.muted));
        return;
    };
    let (lx, ly) = (levels(&x), levels(&y));
    let mut obs = vec![vec![0.0; lx.len()]; ly.len()];
    for (a, b) in x.iter().zip(&y) {
        if let (Some(i), Some(j)) = (ly.iter().position(|v| v == b), lx.iter().position(|v| v == a)) {
            obs[i][j] += 1.0;
        }
    }
    let n: f64 = obs.iter().flatten().sum();
    let rs: Vec<f64> = obs.iter().map(|r| r.iter().sum()).collect();
    let cs: Vec<f64> = (0..lx.len()).map(|j| obs.iter().map(|r| r[j]).sum()).collect();
    let mut text = vec![];
    let mut vals = vec![];
    let mut chi = 0.0;
    for i in 0..ly.len() {
        let mut tr = vec![];
        let mut vr = vec![];
        for j in 0..lx.len() {
            let e = rs[i] * cs[j] / n.max(1.0);
            let resid = if e > 0.0 { (obs[i][j] - e) / e.sqrt() } else { 0.0 };
            chi += resid * resid;
            if mosaic {
                let share = obs[i][j] / cs[j].max(1.0);
                tr.push(format!("{:.0}%", share * 100.0));
                vr.push(share);
            } else {
                tr.push(format!("{:.0} / {:.0}", obs[i][j], e));
                vr.push((resid / 4.0).clamp(-1.0, 1.0));
            }
        }
        text.push(tr);
        vals.push(vr);
    }
    ui.label(RichText::new(if mosaic { format!("Share of {yc} within each {xc}") } else { format!("Observed / expected · χ² = {chi:.2} (blue: more than expected, red: fewer)") }).size(11.5));
    matrix(ui, t, &ly, &lx, &text, &vals, !mosaic);
}

pub fn viz(ui: &mut Ui, t: &Tokens, kind: &str, d: &TableData, s: &BTreeMap<String, J>) {
    let list = |k: &str| -> Vec<String> { s.get(k).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default() };
    match kind {
        "viz.scatter" => {
            let (xc, yc) = (setting(s, "x"), setting(s, "y"));
            let (Some(x), Some(y)) = (num_col(d, &xc), num_col(d, &yc)) else {
                ui.label(RichText::new("Choose numeric x and y columns").color(t.muted));
                return;
            };
            let colors = cat_col(d, &setting(s, "color"));
            let lv = colors.as_ref().map(|c| levels(c)).unwrap_or_default();
            if !lv.is_empty() {
                legend(ui, &lv.iter().enumerate().map(|(i, l)| (l.clone(), color(i))).collect::<Vec<_>>());
            }
            let (p, pl) = frame(ui, t, size(ui, 320.0), range(x.iter().copied()), range(y.iter().copied()), &xc, &yc);
            for i in 0..x.len().min(20_000) {
                if x[i].is_finite() && y[i].is_finite() {
                    let c = colors.as_ref().and_then(|c| lv.iter().position(|l| *l == c[i])).map(color).unwrap_or(color(0));
                    p.circle_filled(pl.px(x[i], y[i]), 2.5, c.gamma_multiply(0.75));
                }
            }
            if s.get("regression").and_then(|v| v.as_bool()).unwrap_or(true) {
                let r = pearson(&x, &y);
                let pts: Vec<(f64, f64)> = x.iter().zip(&y).filter(|(a, b)| a.is_finite() && b.is_finite()).map(|(a, b)| (*a, *b)).collect();
                let n = pts.len() as f64;
                let (mx, my) = (pts.iter().map(|p| p.0).sum::<f64>() / n, pts.iter().map(|p| p.1).sum::<f64>() / n);
                let b = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum::<f64>() / pts.iter().map(|p| (p.0 - mx).powi(2)).sum::<f64>().max(1e-300);
                polyline(&p, &pl, &[(pl.x.0, my + b * (pl.x.0 - mx)), (pl.x.1, my + b * (pl.x.1 - mx))], t.text, 1.5);
                ui.label(RichText::new(format!("r = {r:.3} · slope {b:.4}")).size(11.0));
            }
        }
        "viz.distributions" => {
            let c = setting(s, "column");
            let bins = s.get("bins").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
            let Some(v) = num_col(d, &c).filter(|v| v.iter().any(|x| x.is_finite())) else {
                // Categorical: bar chart of counts.
                if let Some(cv) = cat_col(d, &c) {
                    let mut counts: BTreeMap<String, f64> = BTreeMap::new();
                    for x in cv {
                        *counts.entry(x).or_default() += 1.0;
                    }
                    let mut it: Vec<(String, f64)> = counts.into_iter().collect();
                    it.sort_by(|a, b| b.1.total_cmp(&a.1));
                    bars(ui, t, &it, &c, false);
                } else {
                    ui.label(RichText::new("Choose a column").color(t.muted));
                }
                return;
            };
            let groups = match cat_col(d, &setting(s, "split")) {
                Some(g) => levels(&g).into_iter().map(|l| (l.clone(), v.iter().zip(&g).filter(|(_, x)| **x == l).map(|(a, _)| *a).collect())).collect(),
                None => vec![(c.clone(), v)],
            };
            hist(ui, t, &groups, bins, &c);
        }
        "viz.box" => {
            let c = setting(s, "column");
            let Some(v) = num_col(d, &c) else {
                ui.label(RichText::new("Choose a numeric column").color(t.muted));
                return;
            };
            let groups: Vec<(String, Vec<f64>)> = match cat_col(d, &setting(s, "group")) {
                Some(g) => levels(&g).into_iter().map(|l| (l.clone(), v.iter().zip(&g).filter(|(_, x)| **x == l).map(|(a, _)| *a).collect())).collect(),
                None => vec![(c.clone(), v.clone())],
            };
            let (p, pl) = frame_ex(ui, t, size(ui, 300.0), (-0.5, groups.len() as f64 - 0.5), range(v.iter().copied()), &setting(s, "group"), &c, false);
            for (gi, (name, vals)) in groups.iter().enumerate() {
                let mut x: Vec<f64> = vals.iter().copied().filter(|v| v.is_finite()).collect();
                if x.is_empty() {
                    continue;
                }
                x.sort_by(|a, b| a.total_cmp(b));
                let q = |f: f64| vizcraft_core::stats::quantile(&x, f);
                let (q1, md, q3) = (q(0.25), q(0.5), q(0.75));
                let iqr = q3 - q1;
                let lo = x.iter().copied().find(|v| *v >= q1 - 1.5 * iqr).unwrap_or(q1);
                let hi = x.iter().rev().copied().find(|v| *v <= q3 + 1.5 * iqr).unwrap_or(q3);
                let g = gi as f64;
                let c = color(gi);
                p.rect_filled(Rect::from_two_pos(pl.px(g - 0.25, q1), pl.px(g + 0.25, q3)), 2.0, c.gamma_multiply(0.35));
                p.rect_stroke(Rect::from_two_pos(pl.px(g - 0.25, q1), pl.px(g + 0.25, q3)), 2.0, Stroke::new(1.2, c), StrokeKind::Inside);
                p.line_segment([pl.px(g - 0.25, md), pl.px(g + 0.25, md)], Stroke::new(2.0, c));
                p.line_segment([pl.px(g, q3), pl.px(g, hi)], Stroke::new(1.0, c));
                p.line_segment([pl.px(g, q1), pl.px(g, lo)], Stroke::new(1.0, c));
                for o in x.iter().filter(|v| **v < lo || **v > hi).take(200) {
                    p.circle_stroke(pl.px(g, *o), 2.0, Stroke::new(1.0, c));
                }
                p.text(pl.px(g, pl.y.0) + vec2(0.0, 3.0), Align2::CENTER_TOP, trunc(name, 18), FontId::proportional(10.0), t.text);
            }
        }
        "viz.heatmap" | "viz.correlations" => {
            let cols = numeric_columns(d, &list("columns"));
            if cols.len() < 2 && kind == "viz.correlations" {
                ui.label(RichText::new("Needs at least two numeric columns").color(t.muted));
                return;
            }
            let data: Vec<Vec<f64>> = cols.iter().filter_map(|c| num_col(d, c)).collect();
            if kind == "viz.correlations" {
                let spear = setting(s, "method") == "spearman";
                let data: Vec<Vec<f64>> = if spear { data.iter().map(|v| ranks(v)).collect() } else { data };
                let mut pairs = vec![];
                let mut vals = vec![vec![f64::NAN; cols.len()]; cols.len()];
                for i in 0..cols.len() {
                    for j in 0..cols.len() {
                        let r = if i == j { 1.0 } else { pearson(&data[i], &data[j]) };
                        vals[i][j] = r;
                        if j > i {
                            pairs.push((format!("{} × {}", cols[i], cols[j]), r));
                        }
                    }
                }
                pairs.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
                bars(ui, t, &pairs.into_iter().take(20).collect::<Vec<_>>(), if spear { "Spearman correlations" } else { "Pearson correlations" }, true);
                ui.add_space(8.0);
                let text: Vec<Vec<String>> = vals.iter().map(|r| r.iter().map(|v| format!("{v:.2}")).collect()).collect();
                matrix(ui, t, &cols, &cols, &text, &vals, true);
            } else {
                let max_rows = s.get("max_rows").and_then(|v| v.as_u64()).unwrap_or(200) as usize;
                let n = d.rows.len().min(max_rows).min(400);
                let z: Vec<Vec<f64>> = data
                    .iter()
                    .map(|v| {
                        let f: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
                        let m = f.iter().sum::<f64>() / f.len().max(1) as f64;
                        let sd = (f.iter().map(|x| (x - m).powi(2)).sum::<f64>() / f.len().max(2) as f64).sqrt().max(1e-12);
                        v.iter().take(n).map(|x| ((x - m) / sd / 2.5).clamp(-1.0, 1.0)).collect()
                    })
                    .collect();
                let mut order: Vec<usize> = (0..n).collect();
                if s.get("cluster").and_then(|v| v.as_bool()).unwrap_or(true) && !z.is_empty() {
                    order.sort_by(|a, b| z[0][*a].total_cmp(&z[0][*b]));
                }
                let cw = ((ui.available_width() - 20.0) / cols.len().max(1) as f32).clamp(30.0, 120.0);
                let rh = (300.0 / n.max(1) as f32).clamp(1.0, 14.0);
                let (r, _) = ui.allocate_exact_size(vec2(cw * cols.len() as f32, rh * n as f32 + 20.0), Sense::hover());
                let p = ui.painter_at(r);
                for (j, c) in cols.iter().enumerate() {
                    p.text(r.min + vec2(cw * (j as f32 + 0.5), 8.0), Align2::CENTER_CENTER, trunc(c, 12), FontId::proportional(9.5), t.muted);
                    for (k, &i) in order.iter().enumerate() {
                        let cell = Rect::from_min_size(r.min + vec2(cw * j as f32, 18.0 + rh * k as f32), vec2(cw - 1.0, rh));
                        p.rect_filled(cell, 0.0, heat(t, z[j][i], true));
                    }
                }
            }
        }
        "viz.sieve" | "viz.mosaic" => crosstab(ui, t, d, &setting(s, "x"), &setting(s, "y"), kind == "viz.mosaic"),
        "viz.line" => {
            let xc = setting(s, "x");
            let ys = list("y");
            let ys = if ys.is_empty() { numeric_columns(d, &[]).into_iter().filter(|c| *c != xc).take(3).collect() } else { ys };
            let xv = num_col(d, &xc).filter(|v| v.iter().all(|x| x.is_finite())).unwrap_or_else(|| (0..d.rows.len()).map(|i| i as f64).collect());
            let mut order: Vec<usize> = (0..xv.len()).collect();
            order.sort_by(|a, b| xv[*a].total_cmp(&xv[*b]));
            let series: Vec<(&str, Vec<(f64, f64)>, Color32)> = ys.iter().enumerate().filter_map(|(i, c)| num_col(d, c).map(|v| (c.as_str(), order.iter().map(|&k| (xv[k], v[k])).collect(), color(i)))).collect();
            series_plot(ui, t, &series, &xc, "", 300.0);
        }
        _ => {}
    }
}
