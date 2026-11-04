//! Figma-style shapes: outlines for every shape kind, gradient fills, strokes and rotation.

use egui::epaint::{Mesh, Vertex};
use egui::{Color32, Painter, Pos2, Rect, Stroke, pos2, vec2};
use vizardcraft_core::doc::*;

use crate::theme::c32;

/// The outline of a shape kind inside `r` (screen space, before rotation).
pub fn outline(kind: ShapeKind, r: Rect, radius: f32, points: &[(f32, f32)]) -> Vec<Pos2> {
    let (x0, y0, w, h) = (r.left(), r.top(), r.width(), r.height());
    let at = |fx: f32, fy: f32| pos2(x0 + w * fx, y0 + h * fy);
    let c = r.center();
    match kind {
        ShapeKind::Rectangle | ShapeKind::Frame => rounded(r, radius.max(0.0)),
        ShapeKind::RoundedRectangle => rounded(r, if radius >= 0.0 { radius } else { (w.min(h) * 0.18).min(24.0) }),
        ShapeKind::Ellipse => (0..72).map(|i| {
            let a = i as f32 / 72.0 * std::f32::consts::TAU;
            pos2(c.x + w / 2.0 * a.cos(), c.y + h / 2.0 * a.sin())
        }).collect(),
        ShapeKind::Line => vec![at(0.0, 0.5), at(1.0, 0.5)],
        ShapeKind::Arrow => vec![at(0.0, 0.38), at(0.68, 0.38), at(0.68, 0.12), at(1.0, 0.5), at(0.68, 0.88), at(0.68, 0.62), at(0.0, 0.62)],
        ShapeKind::Triangle => vec![at(0.5, 0.0), at(1.0, 1.0), at(0.0, 1.0)],
        ShapeKind::Diamond => vec![at(0.5, 0.0), at(1.0, 0.5), at(0.5, 1.0), at(0.0, 0.5)],
        ShapeKind::Pentagon => regular(c, w / 2.0, h / 2.0, 5),
        ShapeKind::Hexagon => regular(c, w / 2.0, h / 2.0, 6),
        ShapeKind::Star => (0..10).map(|i| {
            let a = i as f32 / 10.0 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            let rr = if i % 2 == 0 { 1.0 } else { 0.45 };
            pos2(c.x + w / 2.0 * rr * a.cos(), c.y + h / 2.0 * rr * a.sin())
        }).collect(),
        ShapeKind::Chevron => vec![at(0.0, 0.0), at(0.7, 0.0), at(1.0, 0.5), at(0.7, 1.0), at(0.0, 1.0), at(0.3, 0.5)],
        ShapeKind::Speech => {
            let mut pts = rounded(Rect::from_min_max(at(0.0, 0.0), at(1.0, 0.78)), (w.min(h) * 0.12).min(16.0));
            // Tail at the bottom-left.
            let idx = pts.iter().enumerate().filter(|(_, p)| p.y >= y0 + h * 0.77).map(|(i, _)| i).next().unwrap_or(0);
            pts.splice(idx..idx, [at(0.3, 0.78), at(0.18, 1.0), at(0.2, 0.78)]);
            pts
        }
        ShapeKind::Heart => (0..80).map(|i| {
            let t = i as f32 / 80.0 * std::f32::consts::TAU;
            let x = 16.0 * t.sin().powi(3);
            let y = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
            pos2(c.x + x / 34.0 * w, c.y - (y + 2.5) / 32.0 * h)
        }).collect(),
        ShapeKind::Plus => {
            let a = 0.34;
            let b = 0.66;
            vec![at(a, 0.0), at(b, 0.0), at(b, a), at(1.0, a), at(1.0, b), at(b, b), at(b, 1.0), at(a, 1.0), at(a, b), at(0.0, b), at(0.0, a), at(a, a)]
        }
        ShapeKind::Path => points.iter().map(|(fx, fy)| at(*fx, *fy)).collect(),
    }
}

fn regular(c: Pos2, rx: f32, ry: f32, n: usize) -> Vec<Pos2> {
    (0..n).map(|i| {
        let a = i as f32 / n as f32 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        pos2(c.x + rx * a.cos(), c.y + ry * a.sin())
    }).collect()
}

pub fn rounded(r: Rect, radius: f32) -> Vec<Pos2> {
    let rad = radius.min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
    if rad < 0.5 {
        return vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()];
    }
    let mut pts = Vec::new();
    let corners = [(r.right() - rad, r.top() + rad, -90.0f32), (r.right() - rad, r.bottom() - rad, 0.0), (r.left() + rad, r.bottom() - rad, 90.0), (r.left() + rad, r.top() + rad, 180.0)];
    for (cx, cy, start) in corners {
        for i in 0..=8 {
            let a = (start + 90.0 * i as f32 / 8.0).to_radians();
            pts.push(pos2(cx + rad * a.cos(), cy + rad * a.sin()));
        }
    }
    pts
}

/// Rotates points around `c` by `deg` degrees.
pub fn rotate(pts: &mut [Pos2], c: Pos2, deg: f32) {
    if deg == 0.0 {
        return;
    }
    let (s, co) = deg.to_radians().sin_cos();
    for p in pts.iter_mut() {
        let d = *p - c;
        *p = c + vec2(d.x * co - d.y * s, d.x * s + d.y * co);
    }
}

fn stops_color(stops: &[(f32, vizardcraft_core::color::Color)], t: f32) -> Color32 {
    let Some(first) = stops.first() else { return Color32::TRANSPARENT };
    if t <= first.0 {
        return c32(first.1);
    }
    for w in stops.windows(2) {
        if let [a, b] = w
            && t >= a.0
            && t <= b.0
        {
            let f = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 0.0 };
            return c32(a.1.lerp(b.1, f as f64));
        }
    }
    stops.last().map(|s| c32(s.1)).unwrap_or(Color32::TRANSPARENT)
}

/// Fills a polygon with a solid colour or a gradient (vertex colours on a triangulated mesh).
pub fn fill(p: &Painter, pts: &[Pos2], fill: &Fill, bounds: Rect, opacity: f32) {
    if pts.len() < 3 {
        return;
    }
    let tris = crate::paint::triangulate(pts);
    let color_at = |q: Pos2| -> Color32 {
        let c = match fill {
            Fill::None => Color32::TRANSPARENT,
            Fill::Solid(c) => c32(*c),
            Fill::Linear { angle, stops } => {
                let a = angle.to_radians();
                let dir = vec2(a.sin(), -a.cos());
                let half = (bounds.width() * dir.x.abs() + bounds.height() * dir.y.abs()) / 2.0;
                let t = ((q - bounds.center()).dot(dir) / half.max(1e-3) + 1.0) / 2.0;
                stops_color(stops, t.clamp(0.0, 1.0))
            }
            Fill::Radial { stops } => {
                let d = q - bounds.center();
                let t = ((d.x / (bounds.width() / 2.0).max(1e-3)).powi(2) + (d.y / (bounds.height() / 2.0).max(1e-3)).powi(2)).sqrt();
                stops_color(stops, t.clamp(0.0, 1.0))
            }
        };
        c.gamma_multiply(opacity)
    };
    // Gradients look better with a finer mesh: add the centre as a fan for convex shapes.
    let mut mesh = Mesh::default();
    for q in pts {
        mesh.vertices.push(Vertex { pos: *q, uv: egui::epaint::WHITE_UV, color: color_at(*q) });
    }
    for t in tris {
        mesh.indices.extend_from_slice(&t);
    }
    if matches!(fill, Fill::Linear { .. } | Fill::Radial { .. }) {
        // Subdivide each triangle once more for smoother gradients.
        let mut fine = Mesh::default();
        for tri in mesh.indices.chunks(3) {
            let v: Vec<Pos2> = tri.iter().filter_map(|i| mesh.vertices.get(*i as usize).map(|v| v.pos)).collect();
            if let [a, b, c] = v.as_slice() {
                let n = 6;
                for i in 0..n {
                    for j in 0..(n - i) {
                        let pt = |u: f32, w: f32| *a + (*b - *a) * u + (*c - *a) * w;
                        let s = 1.0 / n as f32;
                        let (u, w) = (i as f32 * s, j as f32 * s);
                        let quads = [[pt(u, w), pt(u + s, w), pt(u, w + s)], [pt(u + s, w), pt(u + s, w + s), pt(u, w + s)]];
                        for (k, q) in quads.iter().enumerate() {
                            if k == 1 && i + j + 1 >= n {
                                continue;
                            }
                            let base = fine.vertices.len() as u32;
                            for x in q {
                                fine.vertices.push(Vertex { pos: *x, uv: egui::epaint::WHITE_UV, color: color_at(*x) });
                            }
                            fine.indices.extend_from_slice(&[base, base + 1, base + 2]);
                        }
                    }
                }
            }
        }
        p.add(egui::Shape::mesh(fine));
    } else {
        p.add(egui::Shape::mesh(mesh));
    }
}

pub fn stroke_outline(p: &Painter, pts: &[Pos2], st: &vizardcraft_core::doc::Stroke, closed: bool, scale: f32, opacity: f32) {
    if st.width <= 0.0 || pts.len() < 2 {
        return;
    }
    let s = Stroke::new(st.width * scale, c32(st.color).gamma_multiply(opacity));
    let mut pp = pts.to_vec();
    if closed && let Some(f) = pts.first() {
        pp.push(*f);
    }
    match st.dash.as_str() {
        "dash" => p.extend(egui::Shape::dashed_line(&pp, s, 6.0 * scale, 4.0 * scale)),
        "dot" => p.extend(egui::Shape::dotted_line(&pp, s.color, 4.0 * scale, s.width / 2.0)),
        _ => {
            p.add(egui::Shape::line(pp, s));
        }
    }
}
