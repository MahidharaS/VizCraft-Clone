//! A virtualized data grid: only the visible rows are laid out, so tables of any size scroll
//! smoothly. The header stays in view; clicking a header selects the column, its menu sorts and
//! filters.

use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::theme::Tokens;

pub struct GridCol {
    pub name: String,
    /// Small type glyph shown before the name ("123", "abc", "ƒx", …).
    pub glyph: String,
    /// A column type icon instead of the glyph: (type, calculated, summarized).
    pub ty: Option<(vizardcraft_core::build::ColType, bool, bool)>,
    pub numeric: bool,
    pub width: f32,
    pub selected: bool,
    /// Sorted: Some(ascending).
    pub sorted: Option<bool>,
    pub filtered: bool,
    pub dim: bool,
}

#[derive(Default)]
pub struct GridOut {
    pub header_clicked: Option<usize>,
    /// The header's ▾ button: (column, where to open a menu).
    pub header_menu: Option<(usize, egui::Pos2)>,
    pub cell_clicked: Option<(usize, usize)>,
}

pub const ROW_H: f32 = 24.0;
const HEADER_H: f32 = 30.0;

/// Width that fits the header and some sample values.
pub fn fit_width(ui: &Ui, header: &str, samples: impl Iterator<Item = String>) -> f32 {
    let w = |s: &str, size: f32| ui.ctx().fonts_mut(|f| f.layout_no_wrap(s.to_string(), FontId::proportional(size), Color32::BLACK).size().x);
    let mut best = w(header, 12.5) + 96.0;
    for s in samples.take(60) {
        best = best.max(w(&s, 12.5) + 20.0);
    }
    best.clamp(70.0, 340.0)
}

/// Draws the grid. `cell(row, col)` gives the text of a visible cell.
pub fn show(ui: &mut Ui, t: &Tokens, id: &str, cols: &[GridCol], nrows: usize, cell: &dyn Fn(usize, usize) -> String) -> GridOut {
    let mut out = GridOut::default();
    let num_w = (nrows.max(1).to_string().len() as f32 * 7.5 + 18.0).max(36.0);
    let total_w = num_w + cols.iter().map(|c| c.width).sum::<f32>();
    let total_h = HEADER_H + nrows as f32 * ROW_H;
    let zebra = if t.dark { Color32::from_rgb(0x20, 0x25, 0x2F) } else { Color32::from_rgb(0xFA, 0xFB, 0xFC) };
    let sel_fill = if t.dark { Color32::from_rgba_unmultiplied(0x5B, 0x9B, 0xFF, 28) } else { Color32::from_rgba_unmultiplied(0x2B, 0x6C, 0xE0, 18) };
    egui::ScrollArea::both().id_salt(id).auto_shrink([false, false]).show_viewport(ui, |ui, vp| {
        let (rect, _) = ui.allocate_exact_size(vec2(total_w.max(ui.available_width()), total_h.max(ui.available_height())), Sense::hover());
        let origin = rect.min;
        let painter = ui.painter_at(ui.clip_rect());
        let first = ((vp.min.y - HEADER_H) / ROW_H).floor().max(0.0) as usize;
        let last = (((vp.max.y - HEADER_H) / ROW_H).ceil().max(0.0) as usize + 1).min(nrows);
        let font = FontId::proportional(12.5);
        // Selected column background.
        let mut x = origin.x + num_w;
        let col_x: Vec<f32> = cols
            .iter()
            .map(|c| {
                let x0 = x;
                x += c.width;
                x0
            })
            .collect();
        for r in first..last {
            let y = origin.y + HEADER_H + r as f32 * ROW_H;
            let row_rect = Rect::from_min_size(pos2(origin.x, y), vec2(total_w, ROW_H));
            if r % 2 == 1 {
                painter.rect_filled(row_rect, 0.0, zebra);
            }
            painter.text(pos2(origin.x + num_w - 8.0, y + ROW_H / 2.0), Align2::RIGHT_CENTER, (r + 1).to_string(), FontId::proportional(11.0), t.muted);
            for (ci, c) in cols.iter().enumerate() {
                let x0 = col_x.get(ci).copied().unwrap_or(0.0);
                let cr = Rect::from_min_size(pos2(x0, y), vec2(c.width, ROW_H));
                if !ui.clip_rect().intersects(cr) {
                    continue;
                }
                if c.selected {
                    painter.rect_filled(cr, 0.0, sel_fill);
                }
                let txt = cell(r, ci);
                let p = painter.with_clip_rect(cr.shrink2(vec2(6.0, 0.0)).intersect(ui.clip_rect()));
                let color = if c.dim { t.muted } else { t.text };
                if c.numeric {
                    p.text(pos2(cr.right() - 8.0, cr.center().y), Align2::RIGHT_CENTER, txt, font.clone(), color);
                } else {
                    p.text(pos2(cr.left() + 8.0, cr.center().y), Align2::LEFT_CENTER, txt, font.clone(), color);
                }
                painter.line_segment([cr.right_top(), cr.right_bottom()], Stroke::new(1.0, t.border.gamma_multiply(0.6)));
            }
            painter.line_segment([row_rect.left_bottom(), row_rect.right_bottom()], Stroke::new(1.0, t.border.gamma_multiply(0.5)));
        }
        // Row clicks.
        let resp = ui.interact(Rect::from_min_size(pos2(origin.x, origin.y + HEADER_H), vec2(total_w, nrows as f32 * ROW_H)), ui.scope_id().with((id, "cells")), Sense::click());
        if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            let r = ((p.y - origin.y - HEADER_H) / ROW_H).floor() as usize;
            let c = col_x.iter().zip(cols.iter()).position(|(x0, c)| p.x >= *x0 && p.x < x0 + c.width);
            if let Some(c) = c {
                out.cell_clicked = Some((r, c));
            }
        }
        // Sticky header.
        let hy = origin.y + vp.min.y;
        let header = Rect::from_min_size(pos2(origin.x, hy), vec2(total_w.max(rect.width()), HEADER_H));
        painter.rect_filled(header, 0.0, t.panel_alt);
        painter.line_segment([header.left_bottom(), header.right_bottom()], Stroke::new(1.0, t.border));
        for (ci, c) in cols.iter().enumerate() {
            let x0 = col_x.get(ci).copied().unwrap_or(0.0);
            let hr = Rect::from_min_size(pos2(x0, hy), vec2(c.width, HEADER_H));
            let r = ui.interact(hr, ui.scope_id().with((id, "h", ci)), Sense::click());
            if c.selected {
                painter.rect_filled(hr, 0.0, t.accent_soft);
                painter.line_segment([hr.left_bottom(), hr.right_bottom()], Stroke::new(2.0, t.accent));
            } else if r.hovered() {
                painter.rect_filled(hr, 0.0, t.accent_soft.gamma_multiply(0.6));
            }
            let p = painter.with_clip_rect(hr.intersect(ui.clip_rect()));
            let gcol = if c.dim { t.muted } else { t.accent };
            let gw = match c.ty {
                Some((ty, calc, sum)) => {
                    super::paint_type_icon(&p, Rect::from_center_size(pos2(hr.left() + 17.0, hr.center().y), vec2(18.0, 16.0)), ty, calc, sum, gcol);
                    24.0
                }
                None if c.glyph.is_empty() => 0.0,
                None => {
                    p.text(pos2(hr.left() + 8.0, hr.center().y), Align2::LEFT_CENTER, &c.glyph, FontId::monospace(10.0), gcol);
                    c.glyph.chars().count() as f32 * 6.5 + 6.0
                }
            };
            let name = vizardcraft_core::scene::ellipsize(&c.name, 12.5, c.width - gw - 40.0);
            p.text(pos2(hr.left() + 8.0 + gw, hr.center().y), Align2::LEFT_CENTER, name, FontId::proportional(12.5), if c.dim { t.muted } else { t.text });
            // Sort / filter state and the menu button.
            let mb = Rect::from_center_size(pos2(hr.right() - 12.0, hr.center().y), vec2(18.0, 18.0));
            let mr = ui.interact(mb, ui.scope_id().with((id, "m", ci)), Sense::click());
            if mr.hovered() {
                p.rect_filled(mb, 3.0, t.border);
            }
            let mc = if c.sorted.is_some() || c.filtered { t.accent } else { t.muted };
            match (c.sorted, c.filtered) {
                (Some(true), _) => super::tri(&p, mb.center(), super::Dir::Up, 4.0, mc),
                (Some(false), _) => super::tri(&p, mb.center(), super::Dir::Down, 4.0, mc),
                (None, true) => {
                    // A funnel.
                    let cc = mb.center();
                    p.add(egui::Shape::convex_polygon(vec![cc + vec2(-5.0, -4.0), cc + vec2(5.0, -4.0), cc + vec2(1.0, 1.0), cc + vec2(-1.0, 1.0)], mc, Stroke::NONE));
                    p.rect_filled(Rect::from_min_max(cc + vec2(-1.0, 1.0), cc + vec2(1.0, 5.0)), 0.0, mc);
                }
                _ => super::tri(&p, mb.center(), super::Dir::Down, 3.5, mc),
            }
            if mr.clicked() {
                out.header_menu = Some((ci, mb.left_bottom()));
            } else if r.clicked() {
                out.header_clicked = Some(ci);
            }
            painter.line_segment([hr.right_top(), hr.right_bottom()], Stroke::new(1.0, t.border));
        }
        let corner = Rect::from_min_size(pos2(origin.x, hy), vec2(num_w, HEADER_H));
        painter.rect_filled(corner, 0.0, t.panel_alt);
        painter.line_segment([corner.right_top(), corner.right_bottom()], Stroke::new(1.0, t.border));
    });
    out
}
