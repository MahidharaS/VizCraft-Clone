//! The results grid: virtualized rows, sticky header and row numbers, sortable columns, cell /
//! range selection and copy as TSV.

use egui::{Color32, FontId, Sense, Stroke, Ui, pos2, vec2};
use vizardcraft_core::build::ColType;
use vizardcraft_sql::QueryResult;

use crate::theme::Tokens;

#[derive(Clone, Debug, Default)]
pub struct GridState {
    /// (anchor, end) cells as (row, col).
    pub sel: Option<((usize, usize), (usize, usize))>,
    pub widths: Vec<f32>,
    pub sort: Option<(usize, bool)>,
    /// Column being resized.
    resizing: Option<usize>,
}

impl GridState {
    pub fn selection(&self) -> Option<(std::ops::Range<usize>, std::ops::Range<usize>)> {
        let ((r0, c0), (r1, c1)) = self.sel?;
        Some((r0.min(r1)..r0.max(r1) + 1, c0.min(c1)..c0.max(c1) + 1))
    }
}

const ROW_H: f32 = 22.0;
const HEAD_H: f32 = 40.0;

fn is_numeric(t: ColType) -> bool {
    matches!(t, ColType::Number | ColType::Integer)
}

/// Shows a result; Ctrl+C over the grid copies the selection (or everything) as TSV.
pub fn show(ui: &mut Ui, t: &Tokens, id: egui::Id, r: &mut QueryResult, st: &mut GridState) {
    let ncol = r.columns.len();
    if ncol == 0 {
        ui.add_space(12.0);
        ui.label(egui::RichText::new(match r.rows_affected {
            Some(n) => format!("Done: {n} rows affected ({:.1} ms).", r.elapsed_ms),
            None => format!("Done ({:.1} ms). The statement returned no rows.", r.elapsed_ms),
        }).color(t.muted));
        return;
    }
    let font = FontId::proportional(12.5);
    let mono = FontId::monospace(12.0);
    if st.widths.len() != ncol {
        st.widths = (0..ncol)
            .map(|c| {
                let head = ui.painter().layout_no_wrap(r.columns.get(c).cloned().unwrap_or_default(), FontId::proportional(12.5), t.text).size().x + 34.0;
                let body = (0..r.rows.len().min(200)).map(|i| r.cell_text(i, c).chars().count().min(60) as f32 * 7.2 + 16.0).fold(0.0f32, f32::max);
                head.max(body).clamp(64.0, 340.0)
            })
            .collect();
    }
    let rn_w = (r.rows.len().max(1).to_string().len() as f32 * 8.0 + 18.0).max(40.0);
    let total_w = rn_w + st.widths.iter().sum::<f32>();
    let total_h = HEAD_H + r.rows.len() as f32 * ROW_H;
    let mut sort_req: Option<usize> = None;
    let stripe = if t.dark { Color32::from_white_alpha(5) } else { Color32::from_rgb(0xFA, 0xFB, 0xFC) };
    egui::ScrollArea::both().id_salt(id).auto_shrink([false, false]).show_viewport(ui, |ui, vp| {
        let (rect, resp) = ui.allocate_exact_size(vec2(total_w.max(ui.available_width()), total_h.max(ui.available_height())), Sense::click_and_drag());
        let o = rect.min;
        let p = ui.painter().clone();
        let first = (((vp.min.y - HEAD_H) / ROW_H).floor().max(0.0)) as usize;
        let last = ((vp.max.y / ROW_H).ceil() as usize + 1).min(r.rows.len());
        let sel = st.selection();
        // Column x positions.
        let mut xs = Vec::with_capacity(ncol + 1);
        let mut x = rn_w;
        for w in &st.widths {
            xs.push(x);
            x += w;
        }
        xs.push(x);
        let col_visible = |c: usize| xs[c + 1] >= vp.min.x && xs[c] <= vp.max.x;
        // Body.
        for i in first..last {
            let y = o.y + HEAD_H + i as f32 * ROW_H;
            let row_rect = egui::Rect::from_min_size(pos2(o.x, y), vec2(total_w, ROW_H));
            if i % 2 == 1 {
                p.rect_filled(row_rect, 0.0, stripe);
            }
            for c in 0..ncol {
                if !col_visible(c) {
                    continue;
                }
                let cell = egui::Rect::from_min_max(pos2(o.x + xs[c], y), pos2(o.x + xs[c + 1], y + ROW_H));
                let selected = sel.as_ref().is_some_and(|(rr, cc)| rr.contains(&i) && cc.contains(&c));
                if selected {
                    p.rect_filled(cell, 0.0, t.accent_soft);
                }
                let ty = r.col_types.get(c).copied().unwrap_or_default();
                let v = r.rows.get(i).and_then(|row| row.get(c));
                let null = v.is_none_or(|v| v.is_null());
                let txt = r.cell_text(i, c);
                let shown = vizardcraft_core::scene::ellipsize(&txt, 12.5, cell.width() - 12.0);
                let (align, pos) = if is_numeric(ty) { (egui::Align2::RIGHT_CENTER, cell.right_center() - vec2(6.0, 0.0)) } else { (egui::Align2::LEFT_CENTER, cell.left_center() + vec2(6.0, 0.0)) };
                let color = if null { t.muted.gamma_multiply(0.7) } else { t.text };
                let f = if is_numeric(ty) || matches!(ty, ColType::Date | ColType::DateTime | ColType::Time) { mono.clone() } else { font.clone() };
                p.with_clip_rect(cell.intersect(p.clip_rect())).text(pos, align, if null { "NULL".to_string() } else { shown }, f, color);
                p.line_segment([cell.right_top(), cell.right_bottom()], Stroke::new(1.0, t.border.gamma_multiply(0.6)));
            }
            p.line_segment([pos2(o.x, y + ROW_H), pos2(o.x + total_w, y + ROW_H)], Stroke::new(1.0, t.border.gamma_multiply(0.5)));
        }
        // Sticky header.
        let hy = o.y + vp.min.y;
        let head = egui::Rect::from_min_size(pos2(o.x, hy), vec2(total_w.max(rect.width()), HEAD_H));
        p.rect_filled(head, 0.0, t.panel_alt);
        p.line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0, t.border));
        for c in 0..ncol {
            if !col_visible(c) {
                continue;
            }
            let cell = egui::Rect::from_min_max(pos2(o.x + xs[c], hy), pos2(o.x + xs[c + 1], hy + HEAD_H));
            let ty = r.col_types.get(c).copied().unwrap_or_default();
            let name = r.columns.get(c).cloned().unwrap_or_default();
            let clip = p.with_clip_rect(cell.intersect(p.clip_rect()));
            let name_shown = vizardcraft_core::scene::ellipsize(&name, 12.5, cell.width() - 26.0);
            clip.text(cell.left_top() + vec2(8.0, 6.0), egui::Align2::LEFT_TOP, name_shown, FontId::proportional(12.5), t.text);
            let tname = r.types.get(c).cloned().unwrap_or_default();
            clip.text(cell.left_top() + vec2(8.0, 23.0), egui::Align2::LEFT_TOP, tname.to_lowercase(), FontId::monospace(10.0), type_color(ty, t));
            if let Some((sc, desc)) = st.sort
                && sc == c
            {
                clip.text(cell.right_top() + vec2(-8.0, 8.0), egui::Align2::RIGHT_TOP, if desc { "▼" } else { "▲" }, FontId::proportional(10.0), t.accent);
            }
            p.line_segment([cell.right_top(), cell.right_bottom()], Stroke::new(1.0, t.border));
        }
        // Sticky row numbers.
        let rx = o.x + vp.min.x;
        let rn = egui::Rect::from_min_max(pos2(rx, o.y + HEAD_H + vp.min.y), pos2(rx + rn_w, o.y + vp.max.y.max(HEAD_H)));
        p.rect_filled(rn, 0.0, t.panel_alt);
        for i in first..last {
            let y = o.y + HEAD_H + i as f32 * ROW_H;
            if y + ROW_H < hy + HEAD_H {
                continue;
            }
            let in_sel = sel.as_ref().is_some_and(|(rr, _)| rr.contains(&i));
            p.text(pos2(rx + rn_w - 8.0, y + ROW_H / 2.0), egui::Align2::RIGHT_CENTER, (i + 1).to_string(), FontId::monospace(10.5), if in_sel { t.accent } else { t.muted });
        }
        p.line_segment([pos2(rx + rn_w, hy), pos2(rx + rn_w, o.y + vp.max.y)], Stroke::new(1.0, t.border));
        let corner = egui::Rect::from_min_size(pos2(rx, hy), vec2(rn_w, HEAD_H));
        p.rect_filled(corner, 0.0, t.panel_alt);
        p.text(corner.center(), egui::Align2::CENTER_CENTER, "#", FontId::proportional(11.0), t.muted);

        // Interaction.
        let cell_at = |pos: egui::Pos2| -> Option<(usize, usize)> {
            let lx = pos.x - o.x;
            let ly = pos.y - o.y;
            if ly - vp.min.y < HEAD_H || lx - vp.min.x < rn_w {
                return None;
            }
            let row = ((ly - HEAD_H) / ROW_H).floor() as usize;
            let col = (0..ncol).find(|c| lx >= xs[*c] && lx < xs[*c + 1])?;
            (row < r.rows.len()).then_some((row, col))
        };
        let col_border_at = |pos: egui::Pos2| -> Option<usize> {
            let lx = pos.x - o.x;
            (0..ncol).find(|c| (lx - xs[*c + 1]).abs() < 4.0)
        };
        if let Some(pos) = resp.hover_pos() {
            let in_head = pos.y - o.y - vp.min.y < HEAD_H;
            if in_head && col_border_at(pos).is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
        }
        if resp.drag_started()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let in_head = pos.y - o.y - vp.min.y < HEAD_H;
            if in_head {
                st.resizing = col_border_at(pos);
            } else if let Some(cell) = cell_at(pos) {
                st.sel = Some((cell, cell));
            }
        }
        if resp.dragged() {
            if let Some(c) = st.resizing {
                if let Some(w) = st.widths.get_mut(c) {
                    *w = (*w + resp.drag_delta().x).max(36.0);
                }
            } else if let Some(pos) = resp.interact_pointer_pos()
                && let Some(cell) = cell_at(pos)
                && let Some((a, _)) = st.sel
            {
                st.sel = Some((a, cell));
            }
        }
        if resp.drag_stopped() {
            st.resizing = None;
        }
        if resp.clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let in_head = pos.y - o.y - vp.min.y < HEAD_H;
            if in_head {
                let lx = pos.x - o.x;
                if col_border_at(pos).is_none() {
                    sort_req = (0..ncol).find(|c| lx >= xs[*c] && lx < xs[*c + 1]);
                }
            } else if pos.x - o.x - vp.min.x < rn_w {
                // Row number: select the row.
                let row = ((pos.y - o.y - HEAD_H) / ROW_H).floor() as usize;
                if row < r.rows.len() {
                    st.sel = Some(((row, 0), (row, ncol.saturating_sub(1))));
                }
            } else if let Some(cell) = cell_at(pos) {
                if ui.input(|i| i.modifiers.shift)
                    && let Some((a, _)) = st.sel
                {
                    st.sel = Some((a, cell));
                } else {
                    st.sel = Some((cell, cell));
                }
            }
        }
        if resp.double_clicked()
            && let Some(pos) = resp.interact_pointer_pos()
            && pos.y - o.y - vp.min.y < HEAD_H
            && let Some(c) = col_border_at(pos)
        {
            // Auto-fit.
            let body = (0..r.rows.len().min(500)).map(|i| r.cell_text(i, c).chars().count() as f32 * 7.2 + 16.0).fold(60.0f32, f32::max);
            if let Some(w) = st.widths.get_mut(c) {
                *w = body.min(600.0);
            }
        }
        // Copy (Ctrl+C) when the pointer is over the grid.
        if resp.contains_pointer() || resp.has_focus() {
            let copy = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)));
            if copy {
                let tsv = match st.selection() {
                    Some((rr, cc)) => r.to_tsv(Some(rr), Some(cc)),
                    None => r.to_tsv(None, None),
                };
                ui.ctx().copy_text(tsv);
            }
            if ui.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::A))) {
                st.sel = Some(((0, 0), (r.rows.len().saturating_sub(1), ncol.saturating_sub(1))));
            }
        }
        resp.context_menu(|ui| {
            if ui.button("Copy selection (TSV)").clicked() {
                let tsv = match st.selection() {
                    Some((rr, cc)) => r.to_tsv(Some(rr), Some(cc)),
                    None => r.to_tsv(None, None),
                };
                ui.ctx().copy_text(tsv);
                ui.close();
            }
            if ui.button("Copy all rows").clicked() {
                ui.ctx().copy_text(r.to_tsv(None, None));
                ui.close();
            }
            if ui.button("Select all").clicked() {
                st.sel = Some(((0, 0), (r.rows.len().saturating_sub(1), ncol.saturating_sub(1))));
                ui.close();
            }
        });
    });
    if let Some(c) = sort_req {
        let desc = matches!(st.sort, Some((sc, false)) if sc == c);
        r.sort_by(c, desc);
        st.sort = Some((c, desc));
        st.sel = None;
    }
}

pub fn type_color(ty: ColType, t: &Tokens) -> Color32 {
    match ty {
        ColType::Integer | ColType::Number => Color32::from_rgb(0xB4, 0x53, 0x09),
        ColType::Date | ColType::DateTime | ColType::Time => Color32::from_rgb(0x7C, 0x3A, 0xED),
        ColType::Bool => Color32::from_rgb(0x0E, 0x74, 0x90),
        ColType::Text => if t.dark { Color32::from_rgb(0x4A, 0xDE, 0x80) } else { Color32::from_rgb(0x04, 0x78, 0x57) },
        ColType::Any => t.muted,
    }
}
