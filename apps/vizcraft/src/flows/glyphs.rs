//! Vector icons for flow nodes (drawn with shapes: no font glyphs needed).

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};

pub fn paint(p: &Painter, r: Rect, key: &str, c: Color32) {
    let (x0, y0, w, h) = (r.left(), r.top(), r.width(), r.height());
    let at = |fx: f32, fy: f32| pos2(x0 + w * fx, y0 + h * fy);
    let sw = (w / 13.0).clamp(1.0, 2.2);
    let s = Stroke::new(sw, c);
    let line = |a: Pos2, b: Pos2| p.line_segment([a, b], s);
    let dot = |q: Pos2, rad: f32| p.circle_filled(q, w * rad, c);
    let rect = |a: Pos2, b: Pos2| p.rect_stroke(Rect::from_min_max(a, b), w * 0.06, s, StrokeKind::Middle);
    let fill = |a: Pos2, b: Pos2| p.rect_filled(Rect::from_min_max(a, b), w * 0.04, c);
    let text = |t: &str, size: f32| p.text(r.center(), egui::Align2::CENTER_CENTER, t, egui::FontId::proportional(h * size), c);
    match key {
        "query" | "csv" => {
            // A database cylinder.
            let (l, rr) = (0.18, 0.82);
            p.add(Shape::ellipse_stroke(at(0.5, 0.22), vec2(w * 0.32, h * 0.1), s));
            line(at(l, 0.22), at(l, 0.78));
            line(at(rr, 0.22), at(rr, 0.78));
            let n = 12;
            for (yy, _) in [(0.5, 0), (0.78, 1)] {
                let pts: Vec<Pos2> = (0..=n).map(|i| {
                    let a = std::f32::consts::PI * i as f32 / n as f32;
                    pos2(x0 + w * 0.5 - w * 0.32 * a.cos(), y0 + h * yy + h * 0.1 * a.sin())
                }).collect();
                p.add(Shape::line(pts, s));
            }
            if key == "csv" {
                // A small page corner.
                let _ = 0;
            }
        }
        "sheet" | "table" => {
            rect(at(0.12, 0.18), at(0.88, 0.82));
            line(at(0.12, 0.4), at(0.88, 0.4));
            line(at(0.12, 0.61), at(0.88, 0.61));
            line(at(0.4, 0.18), at(0.4, 0.82));
            if key == "sheet" {
                fill(at(0.12, 0.18), at(0.4, 0.4));
            }
        }
        "columns" => {
            for fx in [0.15, 0.42, 0.69] {
                rect(at(fx, 0.18), at(fx + 0.16, 0.82));
            }
            fill(at(0.42, 0.18), at(0.58, 0.82));
        }
        "filter" => {
            p.add(Shape::convex_polygon(vec![at(0.12, 0.18), at(0.88, 0.18), at(0.58, 0.52), at(0.42, 0.52)], c, Stroke::NONE));
            fill(at(0.43, 0.5), at(0.57, 0.84));
        }
        "sort" => {
            for (i, len) in [0.7, 0.5, 0.3].iter().enumerate() {
                let y = 0.26 + i as f32 * 0.24;
                line(at(0.14, y), at(0.14 + len, y));
            }
            p.arrow(at(0.82, 0.18), vec2(0.0, h * 0.62), s);
        }
        "group" => {
            text("∑", 0.85);
        }
        "join" => {
            p.circle_stroke(at(0.38, 0.5), w * 0.24, s);
            p.circle_stroke(at(0.62, 0.5), w * 0.24, s);
        }
        "concat" => {
            rect(at(0.18, 0.14), at(0.82, 0.44));
            rect(at(0.18, 0.56), at(0.82, 0.86));
        }
        "formula" => {
            text("ƒx", 0.72);
        }
        "impute" => {
            rect(at(0.12, 0.18), at(0.88, 0.82));
            line(at(0.12, 0.5), at(0.88, 0.5));
            line(at(0.5, 0.18), at(0.5, 0.82));
            fill(at(0.54, 0.54), at(0.84, 0.78));
        }
        "normalize" => {
            line(at(0.12, 0.82), at(0.88, 0.82));
            let pts: Vec<Pos2> = (0..=16).map(|i| {
                let x = i as f32 / 16.0;
                let g = (-((x - 0.5) * 4.2).powi(2)).exp();
                at(0.12 + 0.76 * x, 0.8 - 0.62 * g)
            }).collect();
            p.add(Shape::line(pts, s));
        }
        "bin" | "dist" | "stats" => {
            let hs: &[f32] = if key == "stats" { &[0.4, 0.7, 0.55] } else { &[0.3, 0.6, 0.85, 0.5] };
            let n = hs.len() as f32;
            for (i, hh) in hs.iter().enumerate() {
                let x = 0.14 + i as f32 * (0.72 / n);
                fill(at(x, 0.84 - hh * 0.7), at(x + 0.72 / n - 0.04, 0.84));
            }
        }
        "sample" => {
            for (fx, fy, on) in [(0.25, 0.25, true), (0.5, 0.25, false), (0.75, 0.25, true), (0.25, 0.5, false), (0.5, 0.5, true), (0.75, 0.5, false), (0.25, 0.75, true), (0.5, 0.75, false), (0.75, 0.75, true)] {
                if on {
                    dot(at(fx, fy), 0.08);
                } else {
                    p.circle_stroke(at(fx, fy), w * 0.07, Stroke::new(sw * 0.7, c));
                }
            }
        }
        "pivot" | "unpivot" => {
            rect(at(0.12, 0.12), at(0.88, 0.88));
            fill(at(0.12, 0.12), at(0.88, 0.3));
            fill(at(0.12, 0.12), at(0.3, 0.88));
            if key == "pivot" {
                p.arrow(at(0.45, 0.72), vec2(w * 0.3, -h * 0.3), s);
            } else {
                p.arrow(at(0.75, 0.42), vec2(-w * 0.3, h * 0.3), s);
            }
        }
        "dedupe" => {
            rect(at(0.14, 0.14), at(0.64, 0.64));
            rect(at(0.36, 0.36), at(0.86, 0.86));
        }
        "cluster" | "dbscan" | "hclust" => {
            for (cx, cy) in [(0.3, 0.32), (0.7, 0.36), (0.45, 0.72)] {
                for (dx, dy) in [(-0.07, -0.04), (0.06, -0.06), (0.0, 0.07), (0.08, 0.05)] {
                    dot(at(cx + dx, cy + dy), 0.055);
                }
            }
            if key == "dbscan" {
                p.circle_stroke(at(0.45, 0.72), w * 0.2, Stroke::new(sw * 0.7, c));
            }
        }
        "tree" | "forest" => {
            let draw = |ox: f32, sc: f32| {
                let a = |fx: f32, fy: f32| pos2(x0 + w * (ox + fx * sc), y0 + h * (0.1 + fy * 0.8));
                p.line_segment([a(0.5, 0.15), a(0.2, 0.55)], s);
                p.line_segment([a(0.5, 0.15), a(0.8, 0.55)], s);
                p.line_segment([a(0.8, 0.55), a(0.65, 0.92)], s);
                p.line_segment([a(0.8, 0.55), a(0.95, 0.92)], s);
                for q in [a(0.5, 0.15), a(0.2, 0.55), a(0.8, 0.55), a(0.65, 0.92), a(0.95, 0.92)] {
                    p.circle_filled(q, w * 0.06, c);
                }
            };
            if key == "forest" {
                draw(-0.02, 0.55);
                draw(0.44, 0.55);
            } else {
                draw(0.02, 0.9);
            }
        }
        "pca" => {
            p.arrow(at(0.2, 0.8), vec2(w * 0.62, -h * 0.5), s);
            p.arrow(at(0.32, 0.4), vec2(w * 0.2, h * 0.25), s);
            for (fx, fy) in [(0.3, 0.7), (0.45, 0.55), (0.6, 0.5), (0.7, 0.38), (0.5, 0.66)] {
                dot(at(fx, fy), 0.045);
            }
        }
        "linear" | "logistic" => {
            for (fx, fy) in [(0.2, 0.75), (0.32, 0.62), (0.45, 0.6), (0.58, 0.42), (0.72, 0.36), (0.82, 0.22)] {
                dot(at(fx, fy), 0.05);
            }
            if key == "linear" {
                line(at(0.12, 0.82), at(0.9, 0.18));
            } else {
                let pts: Vec<Pos2> = (0..=16).map(|i| {
                    let x = i as f32 / 16.0;
                    let y = 1.0 / (1.0 + (-(x - 0.5) * 10.0).exp());
                    at(0.1 + 0.8 * x, 0.85 - 0.7 * y)
                }).collect();
                p.add(Shape::line(pts, s));
            }
        }
        "bayes" => {
            text("P", 0.8);
        }
        "knn" => {
            dot(at(0.5, 0.5), 0.08);
            p.circle_stroke(at(0.5, 0.5), w * 0.34, Stroke::new(sw * 0.8, c));
            for (fx, fy) in [(0.3, 0.32), (0.72, 0.42), (0.42, 0.75)] {
                dot(at(fx, fy), 0.05);
            }
        }
        "split" => {
            line(at(0.12, 0.5), at(0.45, 0.5));
            line(at(0.45, 0.5), at(0.85, 0.22));
            line(at(0.45, 0.5), at(0.85, 0.78));
            dot(at(0.85, 0.22), 0.07);
            dot(at(0.85, 0.78), 0.07);
        }
        "predict" => {
            p.circle_stroke(at(0.5, 0.5), w * 0.36, s);
            p.circle_stroke(at(0.5, 0.5), w * 0.18, s);
            dot(at(0.5, 0.5), 0.06);
        }
        "score" => {
            rect(at(0.14, 0.14), at(0.86, 0.86));
            p.add(Shape::line(vec![at(0.3, 0.52), at(0.45, 0.67), at(0.72, 0.36)], Stroke::new(sw * 1.3, c)));
        }
        "confusion" | "corr" => {
            for i in 0..3 {
                for j in 0..3 {
                    let a = at(0.14 + i as f32 * 0.25, 0.14 + j as f32 * 0.25);
                    let rr = Rect::from_min_size(a, vec2(w * 0.21, h * 0.21));
                    if (i + j) % 2 == 0 || (key == "confusion" && i == j) {
                        p.rect_filled(rr, 1.5, c);
                    } else {
                        p.rect_stroke(rr, 1.5, Stroke::new(sw * 0.7, c), StrokeKind::Inside);
                    }
                }
            }
        }
        "outlier" => {
            for (fx, fy) in [(0.25, 0.65), (0.35, 0.7), (0.3, 0.55), (0.42, 0.62), (0.38, 0.76)] {
                dot(at(fx, fy), 0.05);
            }
            p.circle_stroke(at(0.75, 0.28), w * 0.1, s);
            dot(at(0.75, 0.28), 0.04);
        }
        "python" => {
            p.text(r.center(), egui::Align2::CENTER_CENTER, "Py", egui::FontId::monospace(h * 0.55), c);
        }
        "save" => {
            // A table with a down arrow into it (lakehouse table).
            rect(at(0.14, 0.42), at(0.86, 0.88));
            line(at(0.14, 0.62), at(0.86, 0.62));
            p.arrow(at(0.5, 0.08), vec2(0.0, h * 0.3), s);
        }
        "export" => {
            rect(at(0.2, 0.3), at(0.8, 0.88));
            p.arrow(at(0.5, 0.62), vec2(0.0, -h * 0.55), s);
        }
        _ => {
            text(&key.chars().next().unwrap_or('?').to_ascii_uppercase().to_string(), 0.7);
        }
    }
}
