//! Paints a core [`Scene`] with egui: the desktop renderer of every visual.

use egui::epaint::{Mesh, PathShape, TextShape, Vertex};
use egui::{Color32, Painter, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use vizardcraft_core::scene::{Align, Item, Line, Scene, Shape};

use crate::theme::c32;

/// Maps scene coordinates to the screen.
#[derive(Clone, Copy, Debug)]
pub struct Xf {
    pub origin: Pos2,
    pub scale: f32,
}

impl Xf {
    pub fn p(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.origin.x + x * self.scale, self.origin.y + y * self.scale)
    }
    pub fn inv(&self, p: Pos2) -> (f32, f32) {
        ((p.x - self.origin.x) / self.scale.max(1e-6), (p.y - self.origin.y) / self.scale.max(1e-6))
    }
}

fn stroke(l: &Option<Line>, xf: &Xf, alpha: f32) -> Stroke {
    match l {
        Some(l) if l.width > 0.0 => Stroke::new(l.width * xf.scale, c32(l.color).gamma_multiply(alpha)),
        _ => Stroke::NONE,
    }
}

pub fn paint_scene(p: &Painter, s: &Scene, xf: Xf) {
    for it in &s.items {
        paint_item(p, it, xf);
    }
    if let Some(m) = &s.message {
        let c = p.ctx().global_style().visuals.weak_text_color();
        let galley = p.layout(m.clone(), egui::FontId::proportional(12.0 * xf.scale.max(0.6)), c, s.width * xf.scale * 0.9);
        let pos = xf.p(s.width / 2.0, s.height / 2.0) - galley.size() / 2.0;
        p.galley(pos, galley, c);
    }
}

fn dashed(p: &Painter, pts: &[Pos2], st: Stroke, dash: (f32, f32), scale: f32) {
    for w in pts.windows(2) {
        if let [a, b] = w {
            p.extend(egui::Shape::dashed_line(&[*a, *b], st, dash.0 * scale, dash.1 * scale));
        }
    }
}

pub fn paint_item(p: &Painter, it: &Item, xf: Xf) {
    let alpha = if it.dimmed { 0.3 } else { 1.0 };
    match &it.shape {
        Shape::Rect { rect, fill, stroke: st, radius } => {
            let r = Rect::from_min_size(xf.p(rect.x, rect.y), vec2(rect.w * xf.scale, rect.h * xf.scale));
            let rad = (radius * xf.scale).min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
            let f = fill.map(|c| c32(c).gamma_multiply(alpha)).unwrap_or(Color32::TRANSPARENT);
            p.rect(r, rad, f, stroke(st, &xf, alpha), StrokeKind::Inside);
        }
        Shape::Circle { cx, cy, r, fill, stroke: st } => {
            let c = xf.p(*cx, *cy);
            if let Some(f) = fill {
                p.circle_filled(c, r * xf.scale, c32(*f).gamma_multiply(alpha));
            }
            let s2 = stroke(st, &xf, alpha);
            if s2.width > 0.0 {
                p.circle_stroke(c, r * xf.scale, s2);
            }
        }
        Shape::Path { points, closed, fill, stroke: st } => {
            let pts: Vec<Pos2> = points.iter().map(|(x, y)| xf.p(*x, *y)).collect();
            if pts.len() < 2 {
                return;
            }
            if *closed && let Some(f) = fill {
                fill_polygon(p, &pts, c32(*f).gamma_multiply(alpha));
            }
            let s2 = stroke(st, &xf, alpha);
            if s2.width > 0.0 {
                if let Some(d) = st.as_ref().and_then(|l| l.dash) {
                    let mut pp = pts.clone();
                    if *closed && let Some(first) = pts.first() {
                        pp.push(*first);
                    }
                    dashed(p, &pp, s2, d, xf.scale);
                } else if *closed {
                    p.add(PathShape::closed_line(pts, s2));
                } else {
                    p.add(PathShape::line(pts, s2));
                }
            }
        }
        Shape::Arc { cx, cy, r_inner, r_outer, start, end, fill, stroke: st } => {
            let c = xf.p(*cx, *cy);
            let (ri, ro) = (r_inner * xf.scale, r_outer * xf.scale);
            let n = (((end - start) / 0.05).ceil() as usize).clamp(2, 400);
            let col = c32(*fill).gamma_multiply(alpha);
            let mut mesh = Mesh::default();
            let pt = |r: f32, a: f32| pos2(c.x + r * a.sin(), c.y - r * a.cos());
            for i in 0..=n {
                let a = start + (end - start) * i as f32 / n as f32;
                mesh.vertices.push(Vertex { pos: pt(ro, a), uv: egui::epaint::WHITE_UV, color: col });
                mesh.vertices.push(Vertex { pos: if ri > 0.0 { pt(ri, a) } else { c }, uv: egui::epaint::WHITE_UV, color: col });
            }
            for i in 0..n as u32 {
                let b = i * 2;
                mesh.indices.extend_from_slice(&[b, b + 1, b + 2, b + 1, b + 3, b + 2]);
            }
            p.add(egui::Shape::mesh(mesh));
            let s2 = stroke(st, &xf, alpha);
            if s2.width > 0.0 && (end - start) < std::f32::consts::TAU - 0.01 {
                p.line_segment([pt(ri.max(0.0), *start).max(c), pt(ro, *start)], s2);
                p.line_segment([pt(ri.max(0.0), *end), pt(ro, *end)], s2);
            }
        }
        Shape::Text { x, y, text, size, color, bold, italic, h, v, angle } => {
            let mut font = egui::FontId::proportional((size * xf.scale).max(1.0));
            if *bold {
                font.family = egui::FontFamily::Name("bold".into());
                if !p.ctx().fonts_mut(|f| f.families().contains(&font.family)) {
                    font.family = egui::FontFamily::Proportional;
                }
            }
            let col = c32(*color).gamma_multiply(alpha);
            let mut job = egui::text::LayoutJob::simple_singleline(text.clone(), font, col);
            if *italic && let Some(s) = job.sections.first_mut() {
                s.format.italics = true;
            }
            let galley = p.ctx().fonts_mut(|f| f.layout_job(job));
            let sz = galley.size();
            let dx = match h {
                Align::Start => 0.0,
                Align::Center => -sz.x / 2.0,
                Align::End => -sz.x,
            };
            let dy = match v {
                Align::Start => 0.0,
                Align::Center => -sz.y / 2.0,
                Align::End => -sz.y,
            };
            let anchor = xf.p(*x, *y);
            if *angle == 0.0 {
                p.galley(anchor + vec2(dx, dy), galley, col);
                if *bold && !p.ctx().fonts_mut(|f| f.families().contains(&egui::FontFamily::Name("bold".into()))) {
                    // Fake bold.
                    let g2 = p.ctx().fonts_mut(|f| f.layout_no_wrap(text.clone(), egui::FontId::proportional((size * xf.scale).max(1.0)), col));
                    p.galley(anchor + vec2(dx + 0.5, dy), g2, col);
                }
            } else {
                let rot = angle.to_radians();
                let off = egui::emath::Rot2::from_angle(rot) * vec2(dx, dy);
                let mut ts = TextShape::new(anchor + off, galley, col);
                ts.angle = rot;
                p.add(ts);
            }
        }
        Shape::Image { rect, .. } => {
            // Images are painted by the caller (they need a texture); draw a frame.
            let r = Rect::from_min_size(xf.p(rect.x, rect.y), vec2(rect.w * xf.scale, rect.h * xf.scale));
            p.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_gray(200)), StrokeKind::Inside);
        }
    }
}

trait MaxPos {
    fn max(self, o: Pos2) -> Pos2;
}
impl MaxPos for Pos2 {
    fn max(self, _o: Pos2) -> Pos2 {
        self
    }
}

/// Fills any simple polygon (triangulated by ear clipping).
pub fn fill_polygon(p: &Painter, pts: &[Pos2], col: Color32) {
    if pts.len() < 3 {
        return;
    }
    let mut mesh = Mesh::default();
    for q in pts {
        mesh.vertices.push(Vertex { pos: *q, uv: egui::epaint::WHITE_UV, color: col });
    }
    for tri in triangulate(pts) {
        mesh.indices.extend_from_slice(&tri);
    }
    p.add(egui::Shape::mesh(mesh));
}

fn area(pts: &[Pos2]) -> f32 {
    let n = pts.len();
    (0..n).map(|i| {
        let a = pts.get(i).copied().unwrap_or(Pos2::ZERO);
        let b = pts.get((i + 1) % n).copied().unwrap_or(Pos2::ZERO);
        a.x * b.y - b.x * a.y
    }).sum::<f32>() / 2.0
}

fn inside(p: Pos2, a: Pos2, b: Pos2, c: Pos2) -> bool {
    let d1 = (p.x - b.x) * (a.y - b.y) - (a.x - b.x) * (p.y - b.y);
    let d2 = (p.x - c.x) * (b.y - c.y) - (b.x - c.x) * (p.y - c.y);
    let d3 = (p.x - a.x) * (c.y - a.y) - (c.x - a.x) * (p.y - a.y);
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

pub fn triangulate(pts: &[Pos2]) -> Vec<[u32; 3]> {
    let n = pts.len();
    let mut idx: Vec<usize> = (0..n).collect();
    if area(pts) < 0.0 {
        idx.reverse();
    }
    let mut out = Vec::new();
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let (ia, ib, ic) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let (a, b, c) = (pts[ia], pts[ib], pts[ic]);
            let cross = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
            if cross <= 0.0 {
                continue;
            }
            if idx.iter().any(|&j| j != ia && j != ib && j != ic && inside(pts[j], a, b, c)) {
                continue;
            }
            out.push([ia as u32, ib as u32, ic as u32]);
            idx.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            // Degenerate: fan the rest.
            break;
        }
    }
    if idx.len() >= 3 {
        for k in 1..idx.len() - 1 {
            out.push([idx[0] as u32, idx[k] as u32, idx[k + 1] as u32]);
        }
    }
    out
}
