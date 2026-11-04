//! Icons drawn in code (original artwork): ribbon buttons, view rail, visualization gallery.

use egui::{Color32, Painter, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use vizcraft_core::doc::VisualKind;

/// The app icon (a bar chart on a gradient tile), rendered at 64×64.
pub fn app_icon() -> egui::IconData {
    let n = 64usize;
    let mut rgba = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let i = (y * n + x) * 4;
            // Rounded tile.
            let (cx, cy) = (x as f32 - 31.5, y as f32 - 31.5);
            let r = 14.0f32;
            let qx = (cx.abs() - (31.5 - r)).max(0.0);
            let qy = (cy.abs() - (31.5 - r)).max(0.0);
            if (qx * qx + qy * qy).sqrt() > r {
                continue;
            }
            let t = (x + y) as f32 / (2 * n) as f32;
            let (r0, g0, b0) = (0x1E as f32, 0x3A as f32, 0x8A as f32);
            let (r1, g1, b1) = (0x14 as f32, 0xB8 as f32, 0xA6 as f32);
            let mut px = [(r0 + (r1 - r0) * t) as u8, (g0 + (g1 - g0) * t) as u8, (b0 + (b1 - b0) * t) as u8, 255u8];
            // Bars.
            let bars = [(14, 38), (26, 26), (38, 32), (50, 16)];
            for (bx, top) in bars {
                if x >= bx - 4 && x < bx + 4 && y >= top && y < 50 {
                    px = [255, 255, 255, 255];
                }
            }
            if let Some(slot) = rgba.get_mut(i..i + 4) {
                slot.copy_from_slice(&px);
            }
        }
    }
    egui::IconData { rgba, width: n as u32, height: n as u32 }
}

fn s(c: Color32) -> Stroke {
    Stroke::new(1.4, c)
}

/// Draws a named icon in `r`.
pub fn paint(p: &Painter, r: Rect, name: &str, c: Color32) {
    let r = r.shrink(r.width() * 0.08);
    let (x0, y0, w, h) = (r.left(), r.top(), r.width(), r.height());
    let at = |fx: f32, fy: f32| pos2(x0 + w * fx, y0 + h * fy);
    let bar = |fx0: f32, fy0: f32, fx1: f32, fy1: f32, col: Color32| p.rect_filled(Rect::from_min_max(at(fx0, fy0), at(fx1, fy1)), 1.0, col);
    let soft = c.gamma_multiply(0.55);
    match name {
        "report" => {
            bar(0.1, 0.55, 0.3, 0.9, c);
            bar(0.4, 0.3, 0.6, 0.9, c);
            bar(0.7, 0.1, 0.9, 0.9, c);
        }
        "table" | "table-view" => {
            p.rect_stroke(Rect::from_min_max(at(0.08, 0.12), at(0.92, 0.88)), 2.0, s(c), StrokeKind::Inside);
            for fy in [0.35, 0.6] {
                p.line_segment([at(0.08, fy), at(0.92, fy)], s(c));
            }
            p.line_segment([at(0.4, 0.12), at(0.4, 0.88)], s(c));
        }
        "model" => {
            for (fx, fy) in [(0.05, 0.08), (0.58, 0.08), (0.3, 0.6)] {
                p.rect_stroke(Rect::from_min_size(at(fx, fy), vec2(w * 0.37, h * 0.3)), 2.0, s(c), StrokeKind::Inside);
            }
            p.line_segment([at(0.42, 0.23), at(0.58, 0.23)], s(c));
            p.line_segment([at(0.25, 0.38), at(0.45, 0.6)], s(c));
        }
        "dax" => {
            p.text(at(0.5, 0.5), egui::Align2::CENTER_CENTER, "ƒx", egui::FontId::proportional(h * 0.62), c);
        }
        "tmdl" => {
            p.text(at(0.5, 0.5), egui::Align2::CENTER_CENTER, "{ }", egui::FontId::monospace(h * 0.55), c);
        }
        "sheet" => {
            p.rect_stroke(Rect::from_min_max(at(0.1, 0.1), at(0.9, 0.9)), 2.0, s(c), StrokeKind::Inside);
            for f in [0.37, 0.63] {
                p.line_segment([at(0.1, f), at(0.9, f)], s(c));
                p.line_segment([at(f, 0.1), at(f, 0.9)], s(c));
            }
            bar(0.1, 0.1, 0.9, 0.37, soft);
        }
        "flow" => {
            for (fx, fy) in [(0.12, 0.25), (0.12, 0.75), (0.5, 0.5), (0.88, 0.5)] {
                p.circle_filled(at(fx, fy), w * 0.1, c);
            }
            p.line_segment([at(0.12, 0.25), at(0.5, 0.5)], s(c));
            p.line_segment([at(0.12, 0.75), at(0.5, 0.5)], s(c));
            p.line_segment([at(0.5, 0.5), at(0.88, 0.5)], s(c));
        }
        "service" | "publish" => {
            p.circle_stroke(at(0.5, 0.5), w * 0.38, s(c));
            p.line_segment([at(0.12, 0.5), at(0.88, 0.5)], s(c));
            p.line_segment([at(0.5, 0.12), at(0.5, 0.88)], s(c));
            p.circle_stroke(at(0.5, 0.5), w * 0.18, s(c));
        }
        "ai" => {
            // A four-point sparkle with a small companion.
            let star = |cx: f32, cy: f32, rr: f32, col: Color32| {
                let k = 0.28;
                let pts = vec![at(cx, cy - rr), at(cx + rr * k, cy - rr * k), at(cx + rr, cy), at(cx + rr * k, cy + rr * k), at(cx, cy + rr), at(cx - rr * k, cy + rr * k), at(cx - rr, cy), at(cx - rr * k, cy - rr * k)];
                crate::paint::fill_polygon(p, &pts, col);
            };
            star(0.42, 0.55, 0.4, c);
            star(0.8, 0.2, 0.17, soft);
        }
        "git" => {
            // A branch: trunk, a fork and a merge, three commits.
            p.line_segment([at(0.3, 0.12), at(0.3, 0.88)], s(c));
            p.add(egui::Shape::line(vec![at(0.3, 0.62), at(0.7, 0.42), at(0.7, 0.3)], s(c)));
            for (fx, fy) in [(0.3, 0.12), (0.3, 0.88), (0.7, 0.3)] {
                p.circle_filled(at(fx, fy), w * 0.1, c);
            }
            p.circle_filled(at(0.3, 0.12), w * 0.05, soft);
        }
        "studio" => {
            // Widgets wired into a model: two inputs, a learner, a scored output.
            p.circle_filled(at(0.15, 0.25), w * 0.09, c);
            p.circle_filled(at(0.15, 0.75), w * 0.09, c);
            p.line_segment([at(0.15, 0.25), at(0.5, 0.5)], s(c));
            p.line_segment([at(0.15, 0.75), at(0.5, 0.5)], s(c));
            p.circle_stroke(at(0.5, 0.5), w * 0.15, s(c));
            p.circle_filled(at(0.5, 0.5), w * 0.06, soft);
            p.line_segment([at(0.65, 0.5), at(0.85, 0.5)], s(c));
            bar(0.82, 0.3, 0.95, 0.7, soft);
        }
        "formulator" => {
            // A data thread: three nodes, a branch, a mini chart at the end.
            p.circle_filled(at(0.15, 0.2), w * 0.08, c);
            p.circle_filled(at(0.15, 0.55), w * 0.08, c);
            p.circle_filled(at(0.45, 0.82), w * 0.08, soft);
            p.line_segment([at(0.15, 0.2), at(0.15, 0.55)], s(c));
            p.line_segment([at(0.15, 0.55), at(0.45, 0.82)], s(soft));
            p.line_segment([at(0.15, 0.2), at(0.45, 0.2)], s(c));
            bar(0.52, 0.28, 0.62, 0.42, c);
            bar(0.68, 0.16, 0.78, 0.42, c);
            bar(0.84, 0.05, 0.94, 0.42, c);
            p.line_segment([at(0.15, 0.55), at(0.5, 0.55)], s(c));
            bar(0.56, 0.6, 0.94, 0.66, soft);
        }
        "story" => {
            for (i, fx) in [0.1, 0.4, 0.7].iter().enumerate() {
                let col = if i == 1 { c } else { soft };
                p.rect_filled(Rect::from_min_size(at(*fx, 0.25), vec2(w * 0.22, h * 0.5)), 2.0, col);
            }
        }
        "dashboard" => {
            bar(0.08, 0.08, 0.55, 0.45, c);
            bar(0.62, 0.08, 0.92, 0.45, soft);
            bar(0.08, 0.55, 0.38, 0.92, soft);
            bar(0.45, 0.55, 0.92, 0.92, c);
        }
        "worksheet" => {
            p.rect_stroke(Rect::from_min_max(at(0.08, 0.08), at(0.92, 0.92)), 2.0, s(c), StrokeKind::Inside);
            bar(0.2, 0.55, 0.35, 0.8, c);
            bar(0.45, 0.35, 0.6, 0.8, c);
            bar(0.7, 0.2, 0.85, 0.8, c);
        }
        "data" | "get-data" => {
            for fy in [0.2, 0.5, 0.8] {
                p.add(egui::Shape::ellipse_stroke(at(0.5, fy), vec2(w * 0.38, h * 0.12), s(c)));
            }
            p.line_segment([at(0.12, 0.2), at(0.12, 0.8)], s(c));
            p.line_segment([at(0.88, 0.2), at(0.88, 0.8)], s(c));
        }
        "sql" => {
            // A database cylinder with a query prompt in front.
            let half = |cy: f32, from: f32, to: f32| -> Vec<Pos2> {
                (0..=12).map(|i| {
                    let a = from + (to - from) * i as f32 / 12.0;
                    pos2(x0 + w * (0.36 + 0.3 * a.cos()), y0 + h * (cy + 0.1 * a.sin()))
                }).collect()
            };
            p.add(egui::Shape::ellipse_stroke(at(0.36, 0.18), vec2(w * 0.3, h * 0.1), s(c)));
            p.line_segment([at(0.06, 0.18), at(0.06, 0.74)], s(c));
            p.line_segment([at(0.66, 0.18), at(0.66, 0.44)], s(c));
            p.add(egui::Shape::line(half(0.46, 0.0, std::f32::consts::PI), s(soft)));
            p.add(egui::Shape::line(half(0.74, std::f32::consts::FRAC_PI_2, std::f32::consts::PI), s(c)));
            let bx = Rect::from_min_max(at(0.46, 0.52), at(0.98, 0.94));
            p.rect_filled(bx, 2.0, c.gamma_multiply(0.18));
            p.rect_stroke(bx, 2.0, s(c), StrokeKind::Inside);
            p.line_segment([at(0.56, 0.62), at(0.66, 0.72)], s(c));
            p.line_segment([at(0.66, 0.72), at(0.56, 0.82)], s(c));
            p.line_segment([at(0.72, 0.84), at(0.88, 0.84)], s(c));
        }
        "transform" => {
            p.rect_stroke(Rect::from_min_max(at(0.08, 0.15), at(0.62, 0.85)), 2.0, s(c), StrokeKind::Inside);
            p.line_segment([at(0.08, 0.38), at(0.62, 0.38)], s(c));
            p.arrow(at(0.66, 0.5), vec2(w * 0.28, 0.0), s(c));
        }
        "refresh" => {
            let pts: Vec<Pos2> = (0..20).map(|i| {
                let a = 0.6 + i as f32 / 19.0 * 5.0;
                pos2(r.center().x + w * 0.35 * a.cos(), r.center().y + h * 0.35 * a.sin())
            }).collect();
            p.add(egui::Shape::line(pts, s(c)));
            p.arrow(at(0.85, 0.35), vec2(0.0, -h * 0.2), s(c));
        }
        "measure" | "calc" => {
            p.text(at(0.5, 0.5), egui::Align2::CENTER_CENTER, "∑", egui::FontId::proportional(h * 0.8), c);
        }
        "column" => {
            p.text(at(0.5, 0.5), egui::Align2::CENTER_CENTER, "⊞", egui::FontId::proportional(h * 0.8), c);
        }
        "text" => {
            p.text(at(0.5, 0.5), egui::Align2::CENTER_CENTER, "A", egui::FontId::proportional(h * 0.85), c);
        }
        "shape" => {
            p.circle_filled(at(0.35, 0.4), w * 0.24, soft);
            p.rect_filled(Rect::from_min_max(at(0.42, 0.45), at(0.9, 0.9)), 2.0, c);
        }
        "image" => {
            p.rect_stroke(Rect::from_min_max(at(0.08, 0.15), at(0.92, 0.85)), 2.0, s(c), StrokeKind::Inside);
            p.add(egui::Shape::convex_polygon(vec![at(0.15, 0.8), at(0.42, 0.45), at(0.62, 0.7), at(0.72, 0.58), at(0.88, 0.8)], soft, Stroke::NONE));
            p.circle_filled(at(0.7, 0.32), w * 0.07, c);
        }
        "button" => {
            p.rect_stroke(Rect::from_min_max(at(0.08, 0.28), at(0.92, 0.72)), 6.0, s(c), StrokeKind::Inside);
            p.line_segment([at(0.3, 0.5), at(0.7, 0.5)], s(c));
        }
        "page" | "new-page" => {
            p.rect_stroke(Rect::from_min_max(at(0.18, 0.08), at(0.82, 0.92)), 2.0, s(c), StrokeKind::Inside);
            p.line_segment([at(0.5, 0.35), at(0.5, 0.65)], s(c));
            p.line_segment([at(0.35, 0.5), at(0.65, 0.5)], s(c));
        }
        "bookmark" => {
            p.add(egui::Shape::convex_polygon(vec![at(0.25, 0.08), at(0.75, 0.08), at(0.75, 0.92), at(0.5, 0.7), at(0.25, 0.92)], soft, s(c)));
        }
        "filter" => {
            p.add(egui::Shape::convex_polygon(vec![at(0.1, 0.12), at(0.9, 0.12), at(0.58, 0.5), at(0.58, 0.88), at(0.42, 0.78), at(0.42, 0.5)], soft, s(c)));
        }
        "theme" => {
            for (i, (fx, fy)) in [(0.32, 0.32), (0.68, 0.32), (0.5, 0.66)].iter().enumerate() {
                let col = [Color32::from_rgb(0x2B, 0x6C, 0xE0), Color32::from_rgb(0x14, 0xB8, 0xA6), Color32::from_rgb(0xF9, 0x73, 0x16)].get(i).copied().unwrap_or(c);
                p.circle_filled(at(*fx, *fy), w * 0.2, col);
            }
        }
        "python" => {
            p.text(at(0.5, 0.5), egui::Align2::CENTER_CENTER, "Py", egui::FontId::monospace(h * 0.55), c);
        }
        "r" => {
            p.text(at(0.5, 0.5), egui::Align2::CENTER_CENTER, "R", egui::FontId::monospace(h * 0.7), c);
        }
        "save" => {
            p.rect_stroke(Rect::from_min_max(at(0.12, 0.12), at(0.88, 0.88)), 2.0, s(c), StrokeKind::Inside);
            bar(0.3, 0.12, 0.7, 0.38, soft);
            bar(0.25, 0.58, 0.75, 0.88, soft);
        }
        "undo" => {
            p.arrow(at(0.8, 0.55), vec2(-w * 0.6, 0.0), s(c));
        }
        "redo" => {
            p.arrow(at(0.2, 0.55), vec2(w * 0.6, 0.0), s(c));
        }
        "parameter" => {
            p.line_segment([at(0.1, 0.5), at(0.9, 0.5)], s(c));
            p.circle_filled(at(0.62, 0.5), w * 0.13, c);
        }
        "relationship" => {
            p.rect_stroke(Rect::from_min_size(at(0.05, 0.3), vec2(w * 0.3, h * 0.4)), 2.0, s(c), StrokeKind::Inside);
            p.rect_stroke(Rect::from_min_size(at(0.65, 0.3), vec2(w * 0.3, h * 0.4)), 2.0, s(c), StrokeKind::Inside);
            p.line_segment([at(0.35, 0.5), at(0.65, 0.5)], s(c));
        }
        "security" => {
            p.add(egui::Shape::convex_polygon(vec![at(0.5, 0.08), at(0.88, 0.22), at(0.8, 0.65), at(0.5, 0.92), at(0.2, 0.65), at(0.12, 0.22)], soft, s(c)));
        }
        "ai" => {
            p.text(at(0.5, 0.5), egui::Align2::CENTER_CENTER, "✦", egui::FontId::proportional(h * 0.85), c);
        }
        "mobile" => {
            p.rect_stroke(Rect::from_min_max(at(0.28, 0.06), at(0.72, 0.94)), 4.0, s(c), StrokeKind::Inside);
            p.circle_filled(at(0.5, 0.84), w * 0.04, c);
        }
        "export" => {
            p.arrow(at(0.5, 0.75), vec2(0.0, -h * 0.55), s(c));
            p.line_segment([at(0.15, 0.85), at(0.85, 0.85)], s(c));
        }
        "open" => {
            p.add(egui::Shape::convex_polygon(vec![at(0.08, 0.25), at(0.4, 0.25), at(0.5, 0.35), at(0.92, 0.35), at(0.92, 0.85), at(0.08, 0.85)], soft, s(c)));
        }
        "new" => {
            p.rect_stroke(Rect::from_min_max(at(0.18, 0.08), at(0.82, 0.92)), 2.0, s(c), StrokeKind::Inside);
        }
        "sparkle" | "quick" => {
            p.text(at(0.5, 0.5), egui::Align2::CENTER_CENTER, "⚡", egui::FontId::proportional(h * 0.8), c);
        }
        _ => {
            if let Some(k) = visual_kind_by_name(name) {
                visual(p, r, k, c);
            } else {
                let ch = name.chars().next().unwrap_or('?').to_ascii_uppercase();
                p.text(r.center(), egui::Align2::CENTER_CENTER, ch, egui::FontId::proportional(h * 0.7), c);
            }
        }
    }
}

fn visual_kind_by_name(n: &str) -> Option<VisualKind> {
    serde_json::from_value(serde_json::Value::String(n.to_string())).ok()
}

/// Small icon for a visual type (gallery buttons).
pub fn visual(p: &Painter, r: Rect, k: VisualKind, c: Color32) {
    use VisualKind::*;
    let (x0, y0, w, h) = (r.left(), r.top(), r.width(), r.height());
    let at = |fx: f32, fy: f32| pos2(x0 + w * fx, y0 + h * fy);
    let a = c;
    let b = Color32::from_rgb(0x14, 0xB8, 0xA6);
    let o = Color32::from_rgb(0xF9, 0x73, 0x16);
    let bar = |fx0: f32, fy0: f32, fx1: f32, fy1: f32, col: Color32| p.rect_filled(Rect::from_min_max(at(fx0, fy0), at(fx1, fy1)), 0.5, col);
    let st = Stroke::new(1.5, a);
    match k {
        StackedColumn | StackedColumn100 => {
            for (i, fx) in [0.12, 0.42, 0.72].iter().enumerate() {
                let top = [0.45, 0.2, 0.35].get(i).copied().unwrap_or(0.3);
                let mid = if k == StackedColumn100 { 0.5 } else { (top + 0.9) / 2.0 };
                let top = if k == StackedColumn100 { 0.1 } else { top };
                bar(*fx, top, fx + 0.18, mid, b);
                bar(*fx, mid, fx + 0.18, 0.9, a);
            }
        }
        ClusteredColumn | Histogram => {
            for (i, fx) in [0.08, 0.38, 0.68].iter().enumerate() {
                let t1 = [0.4, 0.15, 0.3].get(i).copied().unwrap_or(0.3);
                bar(*fx, t1, fx + 0.12, 0.9, a);
                if k == ClusteredColumn {
                    bar(fx + 0.13, t1 + 0.2, fx + 0.25, 0.9, b);
                }
            }
        }
        StackedBar | ClusteredBar | StackedBar100 => {
            for (i, fy) in [0.1, 0.4, 0.7].iter().enumerate() {
                let r1 = [0.55, 0.85, 0.4].get(i).copied().unwrap_or(0.5);
                if k == ClusteredBar {
                    bar(0.08, *fy, r1, fy + 0.1, a);
                    bar(0.08, fy + 0.11, r1 - 0.25, fy + 0.2, b);
                } else {
                    let m = if k == StackedBar100 { 0.5 } else { r1 * 0.6 };
                    let r1 = if k == StackedBar100 { 0.92 } else { r1 };
                    bar(0.08, *fy, m, fy + 0.18, a);
                    bar(m, *fy, r1, fy + 0.18, b);
                }
            }
        }
        Line | Sparkline => {
            p.add(egui::Shape::line(vec![at(0.05, 0.75), at(0.3, 0.45), at(0.5, 0.6), at(0.75, 0.2), at(0.95, 0.35)], st));
            if k == Line {
                p.add(egui::Shape::line(vec![at(0.05, 0.9), at(0.3, 0.7), at(0.55, 0.78), at(0.95, 0.55)], Stroke::new(1.5, o)));
            }
        }
        Area | StackedArea | StackedArea100 => {
            p.add(egui::Shape::convex_polygon(vec![at(0.05, 0.92), at(0.05, 0.6), at(0.35, 0.35), at(0.65, 0.5), at(0.95, 0.2), at(0.95, 0.92)], a.gamma_multiply(0.6), st));
            if k != Area {
                p.add(egui::Shape::convex_polygon(vec![at(0.05, 0.92), at(0.05, 0.78), at(0.5, 0.7), at(0.95, 0.6), at(0.95, 0.92)], b.gamma_multiply(0.8), Stroke::NONE));
            }
        }
        LineStackedColumn | LineClusteredColumn | DualAxis => {
            bar(0.1, 0.5, 0.25, 0.9, a);
            bar(0.42, 0.35, 0.57, 0.9, a);
            bar(0.74, 0.6, 0.89, 0.9, a);
            p.add(egui::Shape::line(vec![at(0.05, 0.4), at(0.5, 0.15), at(0.95, 0.3)], Stroke::new(1.5, o)));
        }
        Ribbon => {
            bar(0.08, 0.3, 0.28, 0.9, a);
            bar(0.72, 0.15, 0.92, 0.9, a);
            p.add(egui::Shape::convex_polygon(vec![at(0.28, 0.3), at(0.72, 0.15), at(0.72, 0.5), at(0.28, 0.6)], b.gamma_multiply(0.6), Stroke::NONE));
        }
        Waterfall => {
            bar(0.06, 0.5, 0.22, 0.9, a);
            bar(0.28, 0.3, 0.44, 0.5, Color32::from_rgb(0x16, 0xA3, 0x4A));
            bar(0.5, 0.3, 0.66, 0.45, Color32::from_rgb(0xDC, 0x26, 0x26));
            bar(0.72, 0.45, 0.88, 0.9, a);
        }
        Funnel => {
            for (i, (l, rr)) in [(0.05, 0.95), (0.18, 0.82), (0.3, 0.7), (0.4, 0.6)].iter().enumerate() {
                let fy = 0.08 + i as f32 * 0.22;
                bar(*l, fy, *rr, fy + 0.18, a);
            }
        }
        Scatter | PackedBubbles => {
            for (fx, fy, rr) in [(0.2, 0.7, 0.07), (0.4, 0.4, 0.1), (0.65, 0.55, 0.06), (0.8, 0.25, 0.12), (0.3, 0.25, 0.05)] {
                p.circle_filled(at(fx, fy), w * rr, if fx > 0.5 { b } else { a });
            }
        }
        Pie | Donut => {
            let cen = r.center();
            let rad = w.min(h) * 0.42;
            let slice = |a0: f32, a1: f32, col: Color32| {
                let mut pts = vec![cen];
                for i in 0..=12 {
                    let t = a0 + (a1 - a0) * i as f32 / 12.0;
                    pts.push(pos2(cen.x + rad * t.sin(), cen.y - rad * t.cos()));
                }
                p.add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
            };
            slice(0.0, 2.6, a);
            slice(2.6, 4.6, b);
            slice(4.6, std::f32::consts::TAU, o);
            if k == Donut {
                p.circle_filled(cen, rad * 0.5, Color32::WHITE);
            }
        }
        Treemap => {
            bar(0.05, 0.05, 0.58, 0.95, a);
            bar(0.6, 0.05, 0.95, 0.55, b);
            bar(0.6, 0.57, 0.95, 0.95, o);
        }
        Map | FilledMap => {
            p.add(egui::Shape::convex_polygon(vec![at(0.1, 0.3), at(0.45, 0.15), at(0.85, 0.3), at(0.9, 0.7), at(0.5, 0.88), at(0.15, 0.7)], a.gamma_multiply(0.25), st));
            p.circle_filled(at(0.4, 0.45), w * 0.08, o);
            p.circle_filled(at(0.68, 0.6), w * 0.05, o);
        }
        Gauge => {
            let cen = at(0.5, 0.78);
            let rad = w * 0.4;
            let pts: Vec<Pos2> = (0..=16).map(|i| {
                let t = std::f32::consts::PI * (1.0 + i as f32 / 16.0);
                pos2(cen.x + rad * t.cos(), cen.y + rad * t.sin())
            }).collect();
            p.add(egui::Shape::line(pts, Stroke::new(w * 0.12, a)));
        }
        Card | MultiRowCard | Kpi => {
            p.text(r.center(), egui::Align2::CENTER_CENTER, if k == Kpi { "▲" } else { "123" }, egui::FontId::proportional(h * 0.42), a);
        }
        Slicer => {
            for (i, fy) in [0.18, 0.5, 0.82].iter().enumerate() {
                let rr = Rect::from_center_size(at(0.18, *fy), vec2(w * 0.16, h * 0.16));
                if i == 1 {
                    p.rect_filled(rr, 1.0, a);
                } else {
                    p.rect_stroke(rr, 1.0, st, StrokeKind::Inside);
                }
                p.line_segment([at(0.35, *fy), at(0.9, *fy)], Stroke::new(1.5, c.gamma_multiply(0.6)));
            }
        }
        Table | Matrix | HighlightTable | HeatMap => {
            for i in 0..3 {
                for j in 0..3 {
                    let rr = Rect::from_min_size(at(0.06 + j as f32 * 0.3, 0.08 + i as f32 * 0.3), vec2(w * 0.28, h * 0.27));
                    let col = match k {
                        HeatMap | HighlightTable => a.gamma_multiply(0.2 + 0.25 * ((i + j) % 3) as f32),
                        _ if i == 0 => a,
                        _ => c.gamma_multiply(0.2),
                    };
                    p.rect_filled(rr, 1.0, col);
                }
            }
        }
        KeyInfluencers | DecompositionTree => {
            p.circle_filled(at(0.15, 0.5), w * 0.1, a);
            for fy in [0.2, 0.5, 0.8] {
                p.line_segment([at(0.15, 0.5), at(0.75, fy)], Stroke::new(1.2, c.gamma_multiply(0.6)));
                p.circle_filled(at(0.8, fy), w * 0.08, b);
            }
        }
        QandA => {
            p.text(r.center(), egui::Align2::CENTER_CENTER, "Q&A", egui::FontId::proportional(h * 0.38), a);
        }
        SmartNarrative => {
            for (i, fy) in [0.2, 0.42, 0.64, 0.86].iter().enumerate() {
                let len = [0.9, 0.75, 0.85, 0.5].get(i).copied().unwrap_or(0.5);
                p.line_segment([at(0.08, *fy), at(len, *fy)], Stroke::new(2.0, if i == 1 { a } else { c.gamma_multiply(0.5) }));
            }
        }
        PythonVisual => {
            p.text(r.center(), egui::Align2::CENTER_CENTER, "Py", egui::FontId::monospace(h * 0.5), a);
        }
        RVisual => {
            p.text(r.center(), egui::Align2::CENTER_CENTER, "R", egui::FontId::monospace(h * 0.6), a);
        }
        BoxPlot => {
            for fx in [0.25, 0.65] {
                p.line_segment([at(fx + 0.08, 0.08), at(fx + 0.08, 0.92)], st);
                p.rect_filled(Rect::from_min_max(at(fx, 0.3), at(fx + 0.16, 0.7)), 1.0, if fx < 0.5 { a } else { b });
            }
        }
        Bullet => {
            bar(0.05, 0.3, 0.95, 0.7, c.gamma_multiply(0.2));
            bar(0.05, 0.42, 0.65, 0.58, a);
            p.line_segment([at(0.78, 0.25), at(0.78, 0.75)], Stroke::new(2.0, o));
        }
        Gantt => {
            bar(0.05, 0.1, 0.45, 0.28, a);
            bar(0.3, 0.4, 0.75, 0.58, b);
            bar(0.6, 0.7, 0.95, 0.88, o);
        }
        WordCloud => {
            p.text(at(0.5, 0.35), egui::Align2::CENTER_CENTER, "Abc", egui::FontId::proportional(h * 0.38), a);
            p.text(at(0.3, 0.75), egui::Align2::CENTER_CENTER, "de", egui::FontId::proportional(h * 0.25), b);
            p.text(at(0.75, 0.72), egui::Align2::CENTER_CENTER, "fg", egui::FontId::proportional(h * 0.2), o);
        }
        Radar => {
            let cen = r.center();
            let pts: Vec<Pos2> = (0..5).map(|i| {
                let t = i as f32 / 5.0 * std::f32::consts::TAU;
                let rr = w * [0.42, 0.3, 0.38, 0.25, 0.35].get(i).copied().unwrap_or(0.3);
                pos2(cen.x + rr * t.sin(), cen.y - rr * t.cos())
            }).collect();
            p.add(egui::Shape::convex_polygon(pts, a.gamma_multiply(0.4), st));
        }
        Sankey => {
            bar(0.05, 0.1, 0.15, 0.55, a);
            bar(0.05, 0.6, 0.15, 0.9, b);
            bar(0.85, 0.2, 0.95, 0.8, o);
            p.add(egui::Shape::convex_polygon(vec![at(0.15, 0.1), at(0.85, 0.2), at(0.85, 0.55), at(0.15, 0.55)], a.gamma_multiply(0.35), Stroke::NONE));
        }
    }
}
