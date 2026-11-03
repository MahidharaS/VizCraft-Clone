//! The sheet grid: virtualized painting (10k+ rows), frozen column letters / header row / row
//! numbers, selection, keyboard navigation, in-cell editing, fill handle, column resizing,
//! context menus and header filters.

use egui::{Align2, Color32, FontId, Key, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use vizcraft_core::doc::DataSheet;
use vizcraft_core::sheet::{self, CellValue};

use super::ops::{self, Range};
use super::{Drag, Edit, FilterPopup, SheetUi};
use crate::app::VizApp;
use crate::theme::Tokens;

pub const RH: f32 = 22.0;
const LETTER_H: f32 = 22.0;
const HEAD_H: f32 = 26.0;
const NUM_W: f32 = 54.0;
pub const DEFAULT_W: f32 = 110.0;
const EXTRA_ROWS: usize = 60;

pub fn col_w(s: &DataSheet, c: usize) -> f32 {
    s.col_widths.get(c).copied().filter(|w| *w >= 8.0).unwrap_or(DEFAULT_W)
}

fn rows_end(st: &SheetUi, s: &DataSheet) -> usize {
    let _ = st;
    s.cells.len().max(1)
}

/// The row `d` steps from `r` in display order (header, visible data rows, empty rows below).
pub fn next_row(st: &SheetUi, r: usize, d: i64) -> usize {
    let vis: &[usize] = st.cache.as_ref().map(|c| c.visible.as_slice()).unwrap_or(&[]);
    let end = vis.last().map(|l| l + 1).unwrap_or(1).max(st.view_rows_end());
    let pos = position_of(vis, end, r);
    let np = (pos as i64 + d).max(0) as usize;
    row_at(vis, end, np)
}

impl SheetUi {
    fn view_rows_end(&self) -> usize {
        self.rows_end_cache
    }
}

fn position_of(vis: &[usize], end: usize, r: usize) -> usize {
    if r == 0 {
        return 0;
    }
    match vis.binary_search(&r) {
        Ok(i) => i + 1,
        Err(i) => {
            if r >= end {
                vis.len() + 1 + (r - end)
            } else {
                i.max(1)
            }
        }
    }
}

fn row_at(vis: &[usize], end: usize, pos: usize) -> usize {
    if pos == 0 {
        0
    } else if let Some(r) = vis.get(pos - 1) {
        *r
    } else {
        end + (pos - 1 - vis.len())
    }
}

struct Layout {
    /// x offsets of columns (content coordinates), len = ncols + 1.
    xs: Vec<f32>,
    ncols: usize,
    /// Body index → row.
    body_rows: usize,
    freeze: bool,
    end: usize,
}

impl Layout {
    fn body_row(&self, vis: &[usize], i: usize) -> usize {
        row_at(vis, self.end, i + usize::from(self.freeze))
    }
    fn body_index(&self, vis: &[usize], r: usize) -> Option<usize> {
        let p = position_of(vis, self.end, r);
        if self.freeze {
            if p == 0 { None } else { Some(p - 1) }
        } else {
            Some(p)
        }
    }
    fn col_at(&self, x: f32) -> usize {
        match self.xs.binary_search_by(|v| v.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Less)) {
            Ok(i) => i.min(self.ncols.saturating_sub(1)),
            Err(i) => i.saturating_sub(1).min(self.ncols.saturating_sub(1)),
        }
    }
    fn x(&self, c: usize) -> f32 {
        self.xs.get(c).copied().unwrap_or(0.0)
    }
    fn w(&self, c: usize) -> f32 {
        self.x(c + 1) - self.x(c)
    }
}

pub fn grid(app: &mut VizApp, st: &mut SheetUi, ui: &mut Ui, t: &Tokens) {
    let Some(s) = app.project.sheets.get(st.active) else { return };
    let grid_id = ui.scope_id().with(("sheet_grid", st.active));
    let used_w = ops::width(s);
    let ncols = (used_w + 8).max(16).max(st.cursor.1 + 4);
    let mut xs = Vec::with_capacity(ncols + 1);
    let mut x = 0.0;
    for c in 0..ncols {
        xs.push(x);
        x += col_w(s, c);
    }
    xs.push(x);
    st.rows_end_cache = rows_end(st, s);
    let vis: Vec<usize> = st.cache.as_ref().map(|c| c.visible.clone()).unwrap_or_default();
    let freeze = st.freeze_header;
    let end = vis.last().map(|l| l + 1).unwrap_or(1).max(st.rows_end_cache);
    let extra = EXTRA_ROWS.max(st.cursor.0.saturating_sub(end) + 10);
    let lay = Layout { xs, ncols, body_rows: vis.len() + extra + usize::from(!freeze), freeze, end };

    // Keyboard first (edit keys must be consumed before the text editors see them).
    let has_focus = ui.memory(|m| m.has_focus(grid_id));
    let any_focus = ui.memory(|m| m.focused().is_some());
    if st.grid_focus && !any_focus && st.edit.is_none() {
        ui.memory_mut(|m| m.request_focus(grid_id));
    }
    if st.edit.is_some() {
        edit_keys(app, st, ui);
    } else if has_focus {
        ui.memory_mut(|m| m.set_focus_lock_filter(grid_id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true }));
        nav_keys(app, st, ui, &vis, &lay);
    }
    let Some(s) = app.project.sheets.get(st.active) else { return };

    let full = ui.available_rect_before_wrap();
    let strip_h = LETTER_H + if freeze { HEAD_H } else { 0.0 };
    let body_rect = Rect::from_min_max(pos2(full.left() + NUM_W, full.top() + strip_h), full.max);
    let total = vec2(lay.x(ncols) + 40.0, lay.body_rows as f32 * RH);

    // Scroll to keep the cursor visible.
    let mut want_offset: Option<egui::Vec2> = None;
    if let Some((r, c)) = st.scroll_to.take() {
        let mut off = st.offset;
        let view = body_rect.size() - vec2(14.0, 14.0);
        if let Some(i) = lay.body_index(&vis, r) {
            let y0 = i as f32 * RH;
            if y0 < off.y {
                off.y = y0;
            } else if y0 + RH > off.y + view.y {
                off.y = y0 + RH - view.y;
            }
        }
        let x0 = lay.x(c);
        let x1 = lay.x(c + 1);
        if x0 < off.x {
            off.x = x0;
        } else if x1 > off.x + view.x {
            off.x = (x1 - view.x).max(0.0);
        }
        if off != st.offset {
            want_offset = Some(off);
        }
    }

    // ---------------------------------------------------------------- body
    let sel = st.selection();
    let values = st.cache.as_ref().map(|c| &c.values);
    let empty: Vec<Vec<CellValue>> = vec![];
    let values = values.unwrap_or(&empty);
    let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body_rect).id_salt("sheet_body"));
    let mut sa = egui::ScrollArea::both().auto_shrink([false, false]).id_salt(("sheet_scroll", st.active));
    if let Some(o) = want_offset {
        sa = sa.scroll_offset(o);
    }
    let mut body_click: Option<(usize, usize, egui::Response)> = None;
    let mut fill_handle_rect = Rect::NOTHING;
    let out = sa.show_viewport(&mut body_ui, |ui, vp| {
        let origin = ui.max_rect().min;
        ui.allocate_rect(Rect::from_min_size(origin, total), Sense::hover());
        let resp = ui.interact(ui.clip_rect(), grid_id, Sense::click_and_drag());
        let painter = ui.painter().clone();
        let i0 = (vp.min.y / RH).floor().max(0.0) as usize;
        let i1 = ((vp.max.y / RH).ceil() as usize).min(lay.body_rows);
        let c0 = lay.col_at(vp.min.x);
        let c1 = (lay.col_at(vp.max.x) + 1).min(ncols);
        let font = FontId::proportional(12.5);
        // Background and selection fills.
        painter.rect_filled(ui.clip_rect(), 0.0, t.panel);
        for i in i0..i1 {
            let r = lay.body_row(&vis, i);
            let y = origin.y + i as f32 * RH;
            let is_header = r == 0;
            if is_header {
                painter.rect_filled(Rect::from_min_size(pos2(origin.x + lay.x(c0), y), vec2(lay.x(c1) - lay.x(c0), RH)), 0.0, t.panel_alt);
            }
            for c in c0..c1 {
                let cell = Rect::from_min_size(pos2(origin.x + lay.x(c), y), vec2(lay.w(c), RH));
                if sel.contains(r, c) && !(r == st.cursor.0 && c == st.cursor.1) {
                    painter.rect_filled(cell, 0.0, sel_fill(t));
                }
                paint_value(&painter, s, values, r, c, cell, &font, t, is_header);
            }
        }
        // Grid lines.
        let grid_col = gridline(t);
        for i in i0..=i1 {
            let y = origin.y + i as f32 * RH;
            painter.line_segment([pos2(origin.x + lay.x(c0), y), pos2(origin.x + lay.x(c1), y)], Stroke::new(1.0, grid_col));
        }
        for c in c0..=c1 {
            let xx = origin.x + lay.x(c);
            painter.line_segment([pos2(xx, origin.y + i0 as f32 * RH), pos2(xx, origin.y + i1 as f32 * RH)], Stroke::new(1.0, grid_col));
        }
        // Selection border and fill handle.
        let ys = |r: usize| lay.body_index(&vis, r).map(|i| origin.y + i as f32 * RH);
        let top = ys(sel.r1).unwrap_or(origin.y - 2.0);
        let bottom = ys(sel.r2).map(|y| y + RH).unwrap_or(origin.y - 2.0);
        if bottom > origin.y - 1.0 {
            let rect = Rect::from_min_max(pos2(origin.x + lay.x(sel.c1), top), pos2(origin.x + lay.x(sel.c2 + 1), bottom));
            painter.rect_stroke(rect, 0.0, Stroke::new(2.0, t.good), StrokeKind::Middle);
            fill_handle_rect = Rect::from_center_size(rect.right_bottom(), vec2(7.0, 7.0));
            painter.rect(fill_handle_rect, 1.0, t.good, Stroke::new(1.0, t.panel), StrokeKind::Outside);
        }
        // Fill preview.
        if let Some(ft) = st.fill_target {
            let top = ys(ft.r1).unwrap_or(origin.y);
            let bottom = ys(ft.r2).map(|y| y + RH).unwrap_or(origin.y);
            let rect = Rect::from_min_max(pos2(origin.x + lay.x(ft.c1), top), pos2(origin.x + lay.x(ft.c2 + 1), bottom));
            dashed_rect(&painter, rect, t.muted);
        }
        // Copy marquee.
        if let Some((_, raw, (r0, c0m))) = &st.clip {
            let h = raw.len();
            let w = raw.first().map(Vec::len).unwrap_or(0);
            if h > 0 && w > 0 && let (Some(a), Some(b)) = (ys(*r0), ys(r0 + h - 1)) {
                let rect = Rect::from_min_max(pos2(origin.x + lay.x(*c0m), a), pos2(origin.x + lay.x(c0m + w), b + RH));
                dashed_rect(&painter, rect.shrink(1.0), t.accent);
            }
        }
        // Pointer → cell.
        if let Some(p) = resp.interact_pointer_pos().or(resp.hover_pos()) {
            let i = ((p.y - origin.y) / RH).floor().max(0.0) as usize;
            let c = lay.col_at(p.x - origin.x);
            let r = lay.body_row(&vis, i.min(lay.body_rows.saturating_sub(1)));
            body_click = Some((r, c, resp));
        }
        origin
    });
    st.offset = out.state.clamped_offset();
    let origin = out.inner;

    // Mouse in the body.
    if let Some((r, c, resp)) = body_click {
        let over_handle = resp.hover_pos().is_some_and(|p| fill_handle_rect.expand(3.0).contains(p));
        if over_handle || st.drag == Drag::Fill {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        } else if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Cell);
        }
        let pressed = resp.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed());
        if pressed {
            st.grid_focus = true;
            ui.memory_mut(|m| m.request_focus(grid_id));
            if over_handle {
                st.drag = Drag::Fill;
                st.fill_target = Some(st.selection());
            } else if point_mode(st) {
                insert_ref(st, r, c);
            } else {
                super::commit_edit(app, st);
                let shift = ui.input(|i| i.modifiers.shift);
                st.select(r, c, shift);
                st.scroll_to = None;
                st.drag = Drag::Select;
            }
        }
        if resp.dragged() {
            match st.drag {
                Drag::Select => {
                    if st.cursor != (r, c) {
                        st.cursor = (r, c);
                        st.scroll_to = Some((r, c));
                    }
                }
                Drag::Fill => {
                    let src = st.selection();
                    let dr = if r > src.r2 { r - src.r2 } else if r < src.r1 { src.r1 - r } else { 0 };
                    let dc = if c > src.c2 { c - src.c2 } else if c < src.c1 { src.c1 - c } else { 0 };
                    st.fill_target = Some(if dr >= dc && dr > 0 {
                        if r > src.r2 { Range { r2: r, ..src } } else { Range { r1: r.max(1), ..src } }
                    } else if dc > 0 {
                        if c > src.c2 { Range { c2: c, ..src } } else { Range { c1: c, ..src } }
                    } else {
                        src
                    });
                    st.scroll_to = Some((r, c));
                }
                _ => {}
            }
        }
        if resp.double_clicked() && !point_mode(st) {
            start_edit(app, st, r, c, None);
        }
        if resp.secondary_clicked() && !st.selection().contains(r, c) {
            st.select(r, c, false);
        }
        resp.context_menu(|ui| cell_menu(app, st, ui));
    }
    if ui.input(|i| i.pointer.any_released()) {
        if st.drag == Drag::Fill
            && let Some(ft) = st.fill_target.take()
        {
            let src = st.selection();
            if ft != src {
                super::apply(app, st, "Fill", |s| ops::fill_series(s, src, ft));
                st.anchor = (ft.r1, ft.c1);
                st.cursor = (ft.r2, ft.c2);
            }
        }
        st.fill_target = None;
        if matches!(st.drag, Drag::Select | Drag::Fill) {
            st.drag = Drag::None;
        }
    }

    // ---------------------------------------------------------------- frozen strips
    let painter = ui.painter().with_clip_rect(full);
    let off = st.offset;
    let sel = st.selection();
    // Column letters.
    let letters = Rect::from_min_size(pos2(body_rect.left(), full.top()), vec2(body_rect.width(), LETTER_H));
    let lp = painter.with_clip_rect(letters);
    lp.rect_filled(letters, 0.0, t.panel_alt);
    let c0 = lay.col_at(off.x);
    let c1 = (lay.col_at(off.x + body_rect.width()) + 1).min(ncols);
    let mut col_actions: Vec<(&str, usize)> = Vec::new();
    for c in c0..c1 {
        let x0 = body_rect.left() + lay.x(c) - off.x;
        let rect = Rect::from_min_size(pos2(x0, full.top()), vec2(lay.w(c), LETTER_H));
        let in_sel = c >= sel.c1 && c <= sel.c2;
        if in_sel {
            lp.rect_filled(rect, 0.0, t.accent_soft);
            lp.line_segment([rect.left_bottom() - vec2(0.0, 1.0), rect.right_bottom() - vec2(0.0, 1.0)], Stroke::new(2.0, t.good));
        }
        lp.text(rect.center(), Align2::CENTER_CENTER, sheet::col_name(c), FontId::proportional(11.5), if in_sel { t.text } else { t.muted });
        lp.line_segment([rect.right_top(), rect.right_bottom()], Stroke::new(1.0, t.border));
        // Click / resize.
        let clipped = rect.intersect(letters);
        if clipped.width() <= 1.0 {
            continue;
        }
        let border = Rect::from_min_max(pos2(rect.right() - 4.0, rect.top()), pos2(rect.right() + 4.0, rect.bottom())).intersect(letters);
        let bresp = ui.interact(border, grid_id.with(("colw", c)), Sense::click_and_drag());
        if bresp.hovered() || bresp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
        }
        if bresp.drag_started() {
            app.checkpoint("Resize column");
            st.drag = Drag::ColResize { col: c, start_w: lay.w(c), start_x: bresp.interact_pointer_pos().map(|p| p.x).unwrap_or(0.0) };
        }
        if bresp.double_clicked() {
            col_actions.push(("autofit", c));
        }
        if bresp.dragged()
            && let Drag::ColResize { col, start_w, start_x } = st.drag
            && let Some(p) = bresp.interact_pointer_pos()
        {
            let w = (start_w + p.x - start_x).clamp(24.0, 900.0);
            if let Some(s) = app.project.sheets.get_mut(st.active) {
                if s.col_widths.len() <= col {
                    s.col_widths.resize(col + 1, 0.0);
                }
                if let Some(cw) = s.col_widths.get_mut(col) {
                    *cw = w;
                }
            }
        }
        if bresp.drag_stopped() {
            st.drag = Drag::None;
        }
        let hit = Rect::from_min_max(clipped.min, pos2(border.left().max(clipped.left()), clipped.bottom()));
        let cresp = ui.interact(hit, grid_id.with(("col", c)), Sense::click());
        if cresp.clicked() {
            col_actions.push(("select", c));
        }
        if cresp.secondary_clicked() && !(c >= sel.c1 && c <= sel.c2 && sel.r1 <= 1) {
            col_actions.push(("select", c));
        }
        cresp.context_menu(|ui| col_menu(app, st, ui, c));
    }
    let Some(s) = app.project.sheets.get(st.active) else { return };
    let last_row = s.cells.len().saturating_sub(1).max(1);
    for (a, c) in col_actions {
        match a {
            "select" => {
                let shift = ui.input(|i| i.modifiers.shift);
                if shift {
                    st.cursor = (last_row, c);
                } else {
                    st.anchor = (0, c);
                    st.cursor = (last_row, c);
                }
                st.grid_focus = true;
            }
            "autofit" => {
                let Some(s) = app.project.sheets.get(st.active) else { continue };
                let values = st.cache.as_ref().map(|c| c.values.as_slice()).unwrap_or(&[]);
                let mut w: f32 = 40.0;
                for r in 0..s.cells.len().min(2000) {
                    let txt = super::display_value(s, values, r, c);
                    w = w.max(crate::widgets::text_w(ui, &txt) * 1.1 + 18.0);
                }
                let w = w.min(600.0);
                super::apply(app, st, "Autofit column", |s| {
                    if s.col_widths.len() <= c {
                        s.col_widths.resize(c + 1, 0.0);
                    }
                    if let Some(cw) = s.col_widths.get_mut(c) {
                        *cw = w;
                    }
                });
            }
            _ => {}
        }
    }
    let Some(s) = app.project.sheets.get(st.active) else { return };
    let values = st.cache.as_ref().map(|c| &c.values).unwrap_or(&empty);
    // Frozen header row.
    let mut head_click: Option<(usize, egui::Response)> = None;
    if freeze {
        let head = Rect::from_min_size(pos2(body_rect.left(), full.top() + LETTER_H), vec2(body_rect.width(), HEAD_H));
        let hp = painter.with_clip_rect(head);
        hp.rect_filled(head, 0.0, t.panel_alt);
        let font = FontId::proportional(12.5);
        let filters = st.filters.get(&s.name);
        for c in c0..c1 {
            let x0 = body_rect.left() + lay.x(c) - off.x;
            let rect = Rect::from_min_size(pos2(x0, head.top()), vec2(lay.w(c), HEAD_H));
            if sel.contains(0, c) && st.cursor != (0, c) {
                hp.rect_filled(rect, 0.0, sel_fill(t));
            }
            paint_value(&hp, s, values, 0, c, rect, &font, t, true);
            hp.line_segment([rect.right_top(), rect.right_bottom()], Stroke::new(1.0, gridline(t)));
            if st.filter_buttons && c < ops::width(s) && !ops::get(s, 0, c).is_empty() {
                let active = filters.and_then(|f| f.get(&c)).is_some_and(|h| !h.is_empty());
                let b = Rect::from_center_size(pos2(rect.right() - 11.0, rect.center().y), vec2(16.0, 16.0));
                let br = ui.interact(b.intersect(head), grid_id.with(("filter", c)), Sense::click());
                hp.rect(b, 3.0, if active { t.accent } else if br.hovered() { t.accent_soft } else { t.panel }, Stroke::new(1.0, if active { t.accent } else { t.border }), StrokeKind::Inside);
                let cc = b.center();
                hp.add(egui::Shape::convex_polygon(vec![cc + vec2(-3.5, -1.5), cc + vec2(3.5, -1.5), cc + vec2(0.0, 2.5)], if active { Color32::WHITE } else { t.muted }, Stroke::NONE));
                if br.on_hover_text("Filter and sort").clicked() {
                    st.filter_popup = Some(FilterPopup { col: c, pos: b.left_bottom() + vec2(-180.0, 4.0), search: String::new(), values: vec![] });
                }
            }
        }
        hp.line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.5, t.border));
        let hr = ui.interact(Rect::from_min_max(head.min, pos2(head.right(), head.bottom())), grid_id.with("head"), Sense::click_and_drag());
        if let Some(p) = hr.interact_pointer_pos().or(hr.hover_pos()) {
            let c = lay.col_at(p.x - body_rect.left() + off.x);
            head_click = Some((c, hr));
        }
        // Selection outline crossing into the header row.
        if sel.r1 == 0 {
            let x0 = body_rect.left() + lay.x(sel.c1) - off.x;
            let x1 = body_rect.left() + lay.x(sel.c2 + 1) - off.x;
            let rect = Rect::from_min_max(pos2(x0, head.top()), pos2(x1, head.bottom() + if sel.r2 > 0 { 2.0 } else { 0.0 }));
            hp.rect_stroke(rect, 0.0, Stroke::new(2.0, t.good), StrokeKind::Inside);
        }
    }
    if let Some((c, hr)) = head_click {
        if hr.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed()) {
            st.grid_focus = true;
            ui.memory_mut(|m| m.request_focus(grid_id));
            if point_mode(st) {
                insert_ref(st, 0, c);
            } else {
                super::commit_edit(app, st);
                let shift = ui.input(|i| i.modifiers.shift);
                st.select(0, c, shift);
                st.drag = Drag::Select;
            }
        }
        if hr.double_clicked() {
            start_edit(app, st, 0, c, None);
        }
        if hr.secondary_clicked() && !st.selection().contains(0, c) {
            st.select(0, c, false);
        }
        hr.context_menu(|ui| cell_menu(app, st, ui));
    }
    // Row numbers.
    let nums = Rect::from_min_max(pos2(full.left(), body_rect.top()), pos2(body_rect.left(), full.bottom()));
    let np = painter.with_clip_rect(nums);
    np.rect_filled(nums, 0.0, t.panel_alt);
    let i0 = (off.y / RH).floor().max(0.0) as usize;
    let i1 = (((off.y + body_rect.height()) / RH).ceil() as usize).min(lay.body_rows);
    let mut row_action: Option<usize> = None;
    for i in i0..i1 {
        let r = lay.body_row(&vis, i);
        let y = body_rect.top() + i as f32 * RH - off.y;
        let rect = Rect::from_min_size(pos2(full.left(), y), vec2(NUM_W, RH));
        let in_sel = r >= sel.r1 && r <= sel.r2;
        if in_sel {
            np.rect_filled(rect, 0.0, t.accent_soft);
            np.line_segment([rect.right_top() - vec2(1.0, 0.0), rect.right_bottom() - vec2(1.0, 0.0)], Stroke::new(2.0, t.good));
        }
        let filtered_gap = i > 0 && r > 1 && lay.body_row(&vis, i - 1) + 1 != r && r < lay.end;
        np.text(rect.center(), Align2::CENTER_CENTER, (r + 1).to_string(), FontId::proportional(11.0), if filtered_gap { t.accent } else if in_sel { t.text } else { t.muted });
        np.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, t.border));
        let rr = ui.interact(rect.intersect(nums), grid_id.with(("row", i)), Sense::click());
        if rr.clicked() || (rr.secondary_clicked() && !(r >= sel.r1 && r <= sel.r2)) {
            row_action = Some(r);
        }
        rr.context_menu(|ui| row_menu(app, st, ui));
    }
    np.line_segment([nums.right_top(), nums.right_bottom()], Stroke::new(1.0, t.border));
    if let Some(r) = row_action {
        let w = app.project.sheets.get(st.active).map(|s| ops::width(s)).unwrap_or(1).max(1) - 1;
        let shift = ui.input(|i| i.modifiers.shift);
        if !shift {
            st.anchor = (r, 0);
        }
        st.cursor = (r, w);
        st.grid_focus = true;
    }
    // Corner (select all) and the frozen header's row number.
    let corner = Rect::from_min_size(full.min, vec2(NUM_W, strip_h));
    painter.rect_filled(corner, 0.0, t.panel_alt);
    let cr = ui.interact(Rect::from_min_size(full.min, vec2(NUM_W, LETTER_H)), grid_id.with("corner"), Sense::click()).on_hover_text("Select all");
    painter.add(egui::Shape::convex_polygon(vec![corner.left_top() + vec2(NUM_W - 6.0, LETTER_H - 14.0), corner.left_top() + vec2(NUM_W - 6.0, LETTER_H - 4.0), corner.left_top() + vec2(NUM_W - 16.0, LETTER_H - 4.0)], t.border, Stroke::NONE));
    if cr.clicked() {
        let Some(s) = app.project.sheets.get(st.active) else { return };
        st.anchor = (0, 0);
        st.cursor = (s.cells.len().saturating_sub(1), ops::width(s).saturating_sub(1));
    }
    if freeze {
        let hr = Rect::from_min_size(pos2(full.left(), full.top() + LETTER_H), vec2(NUM_W, HEAD_H));
        painter.text(hr.center(), Align2::CENTER_CENTER, "1", FontId::proportional(11.0), if sel.r1 == 0 { t.text } else { t.muted });
        painter.line_segment([hr.left_bottom(), hr.right_bottom()], Stroke::new(1.5, t.border));
        let rr = ui.interact(hr, grid_id.with("row0"), Sense::click());
        if rr.clicked() {
            let Some(s) = app.project.sheets.get(st.active) else { return };
            st.anchor = (0, 0);
            st.cursor = (0, ops::width(s).saturating_sub(1));
        }
    }
    painter.line_segment([pos2(full.left(), full.top() + LETTER_H), pos2(full.right(), full.top() + LETTER_H)], Stroke::new(1.0, t.border));
    painter.line_segment([corner.right_top(), corner.right_bottom()], Stroke::new(1.0, t.border));

    // ---------------------------------------------------------------- in-cell editor
    if let Some(e) = st.edit.clone() {
        let rect = if e.r == 0 && freeze {
            Some(Rect::from_min_size(pos2(body_rect.left() + lay.x(e.c) - off.x, full.top() + LETTER_H), vec2(lay.w(e.c), HEAD_H)))
        } else {
            lay.body_index(&vis, e.r).map(|i| Rect::from_min_size(pos2(origin.x + lay.x(e.c), origin.y + i as f32 * RH), vec2(lay.w(e.c), RH)))
        };
        if let Some(rect) = rect.filter(|r| r.intersects(full)) {
            let s_text_w = crate::widgets::text_w(ui, &e.text) * 1.1 + 24.0;
            let rect = Rect::from_min_size(rect.min, vec2(rect.width().max(s_text_w).min(full.right() - rect.left()), rect.height()));
            ui.painter().rect(rect.expand(1.0), 0.0, t.panel, Stroke::new(2.0, t.good), StrokeKind::Inside);
            let id = ui.scope_id().with("cell_edit");
            let mut text = e.text.clone();
            if e.focus_cell {
                ui.memory_mut(|m| m.request_focus(id));
                // Caret at the end.
                let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
                state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(text.chars().count()))));
                state.store(ui.ctx(), id);
            }
            let te = egui::TextEdit::singleline(&mut text).id(id).frame(egui::Frame::NONE).font(FontId::proportional(12.5)).margin(egui::Margin::symmetric(5, 4)).desired_width(rect.width() - 8.0);
            let r = ui.put(rect.shrink(1.0), te);
            if let Some(ed) = st.edit.as_mut() {
                ed.text = text;
                ed.focus_cell = false;
            }
            if r.lost_focus() && !ui.memory(|m| m.focused().is_some()) && st.edit.is_some() && !ui.input(|i| i.pointer.any_pressed()) {
                // Focus left for nowhere (e.g. a click on the canvas is handled above).
                super::commit_edit(app, st);
                st.grid_focus = true;
            }
        }
    }
    // Formula references of the cursor cell, highlighted.
    let _ = (Modifiers::NONE, Key::A);
}

fn sel_fill(t: &Tokens) -> Color32 {
    if t.dark { Color32::from_rgba_unmultiplied(0x5B, 0x9B, 0xFF, 40) } else { Color32::from_rgba_unmultiplied(0x2B, 0x6C, 0xE0, 26) }
}

fn gridline(t: &Tokens) -> Color32 {
    if t.dark { Color32::from_rgb(0x2A, 0x2F, 0x3B) } else { Color32::from_rgb(0xE6, 0xE9, 0xEE) }
}

fn dashed_rect(p: &egui::Painter, r: Rect, c: Color32) {
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    for w in pts.windows(2) {
        if let [a, b] = w {
            p.extend(egui::Shape::dashed_line(&[*a, *b], Stroke::new(1.5, c), 5.0, 3.0));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_value(p: &egui::Painter, s: &DataSheet, values: &[Vec<CellValue>], r: usize, c: usize, cell: Rect, font: &FontId, t: &Tokens, header: bool) {
    let v = values.get(r).and_then(|row| row.get(c));
    let Some(v) = v else { return };
    if matches!(v, CellValue::Empty) {
        return;
    }
    let fmt = super::cell_format(s, r, c);
    let txt = v.display(&fmt);
    let (align, pos, color) = match v {
        CellValue::Num(_) if !header => (Align2::RIGHT_CENTER, cell.right_center() - vec2(6.0, 0.0), t.text),
        CellValue::Bool(_) => (Align2::CENTER_CENTER, cell.center(), t.text),
        CellValue::Error(_) => (Align2::CENTER_CENTER, cell.center(), t.danger),
        _ => (Align2::LEFT_CENTER, cell.left_center() + vec2(6.0, 0.0), t.text),
    };
    let clip = p.with_clip_rect(cell.shrink2(vec2(2.0, 0.0)).intersect(p.clip_rect()));
    if header {
        clip.text(pos, align, txt, FontId::new(font.size, egui::FontFamily::Proportional), t.text);
        // Bold-ish: draw twice with a half-pixel offset.
        let _ = color;
        return;
    }
    let is_formula = ops::get(s, r, c).starts_with('=');
    clip.text(pos, align, txt, font.clone(), color);
    if is_formula && !matches!(v, CellValue::Error(_)) {
        // A small marker for formula cells.
        p.add(egui::Shape::convex_polygon(vec![cell.left_top() + vec2(1.0, 1.0), cell.left_top() + vec2(6.0, 1.0), cell.left_top() + vec2(1.0, 6.0)], Color32::from_rgba_unmultiplied(t.good.r(), t.good.g(), t.good.b(), 150), Stroke::NONE));
    }
}

// =============================================================================================
// Editing
// =============================================================================================

fn point_mode(st: &SheetUi) -> bool {
    st.edit.as_ref().is_some_and(|e| e.text.starts_with('=') && e.text.trim_end().chars().last().is_some_and(|c| "=+-*/^&(,:<>;".contains(c)))
}

fn insert_ref(st: &mut SheetUi, r: usize, c: usize) {
    if let Some(e) = st.edit.as_mut() {
        e.text.push_str(&sheet::address(r, c));
        e.focus_cell = true;
    }
}

pub fn start_edit(app: &VizApp, st: &mut SheetUi, r: usize, c: usize, typed: Option<String>) {
    let Some(s) = app.project.sheets.get(st.active) else { return };
    let original = ops::get(s, r, c).to_string();
    st.anchor = (r, c);
    st.cursor = (r, c);
    st.scroll_to = Some((r, c));
    let (text, typed_mode) = match typed {
        Some(t) => (t, true),
        None => (original.clone(), false),
    };
    st.edit = Some(Edit { r, c, text, original, typed: typed_mode, focus_cell: true, focus_bar: false });
    st.grid_focus = false;
}

fn edit_keys(app: &mut VizApp, st: &mut SheetUi, ui: &mut Ui) {
    let typed = st.edit.as_ref().is_some_and(|e| e.typed);
    let (enter, shift_enter, tab, shift_tab, esc, arrows) = ui.input_mut(|i| {
        let shift_enter = i.consume_key(Modifiers::SHIFT, Key::Enter);
        let enter = i.consume_key(Modifiers::NONE, Key::Enter);
        let shift_tab = i.consume_key(Modifiers::SHIFT, Key::Tab);
        let tab = i.consume_key(Modifiers::NONE, Key::Tab);
        let esc = i.consume_key(Modifiers::NONE, Key::Escape);
        let mut arrows = (0i64, 0i64);
        if typed {
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                arrows = (1, 0);
            } else if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                arrows = (-1, 0);
            } else if i.consume_key(Modifiers::NONE, Key::ArrowLeft) {
                arrows = (0, -1);
            } else if i.consume_key(Modifiers::NONE, Key::ArrowRight) {
                arrows = (0, 1);
            }
        }
        (enter, shift_enter, tab, shift_tab, esc, arrows)
    });
    // In point mode, arrows shouldn't commit a formula such as "=A1+".
    if esc {
        st.edit = None;
        st.grid_focus = true;
        return;
    }
    let (r, c) = st.cursor;
    let mv = if enter {
        Some((1, 0))
    } else if shift_enter {
        Some((-1, 0))
    } else if tab {
        Some((0, 1))
    } else if shift_tab {
        Some((0, -1))
    } else if arrows != (0, 0) {
        Some(arrows)
    } else {
        None
    };
    if let Some((dr, dc)) = mv {
        super::commit_edit(app, st);
        let nr = if dr != 0 { next_row(st, r, dr) } else { r };
        let nc = (c as i64 + dc).max(0) as usize;
        st.select(nr, nc, false);
        st.grid_focus = true;
    }
}

fn nav_keys(app: &mut VizApp, st: &mut SheetUi, ui: &mut Ui, vis: &[usize], lay: &Layout) {
    let _ = lay;
    let events = ui.input(|i| i.events.clone());
    let mods = ui.input(|i| i.modifiers);
    for ev in &events {
        match ev {
            egui::Event::Copy => super::copy(app, st, ui.ctx(), false),
            egui::Event::Cut => super::copy(app, st, ui.ctx(), true),
            egui::Event::Paste(text) => super::paste_text(app, st, text),
            egui::Event::Text(txt) if !mods.command && !mods.ctrl => {
                let (r, c) = st.cursor;
                ui.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(_))));
                start_edit(app, st, r, c, Some(txt.clone()));
                return;
            }
            _ => {}
        }
    }
    let Some(s) = app.project.sheets.get(st.active) else { return };
    let w = ops::width(s).max(1);
    let last = s.cells.len().saturating_sub(1);
    let (r, c) = st.cursor;
    let page = ((ui.available_height() / RH) as i64 - 3).max(5);
    st.view_rows = page as usize;
    let key = |k: Key, m: Modifiers| ui.input_mut(|i| i.consume_key(m, k));
    let empty_at = |rr: usize, cc: usize| ops::get(s, rr, cc).is_empty();
    // Ctrl+arrow: jump to the edge of the data block.
    let jump_v = |dir: i64| -> usize {
        let mut row = r;
        let step = |x: usize| next_row(st, x, dir);
        let n = step(row);
        if n == row {
            return row;
        }
        if empty_at(n, c) || empty_at(row, c) {
            // Next non-empty, or the edge.
            let mut x = n;
            loop {
                if !empty_at(x, c) {
                    return x;
                }
                let y = step(x);
                if y == x || (dir > 0 && y > last) {
                    return if dir > 0 { last.max(r) } else { 0 };
                }
                x = y;
            }
        }
        loop {
            let y = step(row);
            if y == row || empty_at(y, c) || (dir > 0 && y > last) {
                return row;
            }
            row = y;
        }
    };
    let jump_h = |dir: i64| -> usize {
        let mut col = c as i64;
        let ok = |x: i64| x >= 0 && (x as usize) < w + 1;
        let n = col + dir;
        if !ok(n) {
            return c;
        }
        if empty_at(r, n as usize) || empty_at(r, c) {
            let mut x = n;
            while ok(x) {
                if !empty_at(r, x as usize) {
                    return x as usize;
                }
                x += dir;
            }
            return if dir > 0 { w - 1 } else { 0 };
        }
        while ok(col + dir) && !empty_at(r, (col + dir) as usize) {
            col += dir;
        }
        col as usize
    };
    let mut target: Option<((usize, usize), bool)> = None;
    for (k, dr, dc) in [(Key::ArrowDown, 1i64, 0i64), (Key::ArrowUp, -1, 0), (Key::ArrowRight, 0, 1), (Key::ArrowLeft, 0, -1)] {
        for (m, extend, jump) in [(Modifiers::COMMAND | Modifiers::SHIFT, true, true), (Modifiers::COMMAND, false, true), (Modifiers::SHIFT, true, false), (Modifiers::NONE, false, false)] {
            if key(k, m) {
                let nr = if dr != 0 { if jump { jump_v(dr) } else { next_row(st, r, dr) } } else { r };
                let nc = if dc != 0 { if jump { jump_h(dc) } else { (c as i64 + dc).max(0) as usize } } else { c };
                target = Some(((nr, nc), extend));
            }
        }
    }
    if key(Key::Tab, Modifiers::SHIFT) {
        target = Some(((r, c.saturating_sub(1)), false));
    }
    if key(Key::Tab, Modifiers::NONE) {
        target = Some(((r, c + 1), false));
    }
    if key(Key::Enter, Modifiers::SHIFT) {
        target = Some(((next_row(st, r, -1), c), false));
    }
    if key(Key::Enter, Modifiers::NONE) {
        target = Some(((next_row(st, r, 1), c), false));
    }
    if key(Key::Home, Modifiers::COMMAND) {
        target = Some(((if st.freeze_header { 1 } else { 0 }, 0), false));
    }
    if key(Key::Home, Modifiers::SHIFT) {
        target = Some(((r, 0), true));
    }
    if key(Key::Home, Modifiers::NONE) {
        target = Some(((r, 0), false));
    }
    if key(Key::End, Modifiers::COMMAND) {
        target = Some(((vis.last().copied().unwrap_or(last), w - 1), false));
    }
    if key(Key::End, Modifiers::SHIFT) {
        target = Some(((r, w - 1), true));
    }
    if key(Key::End, Modifiers::NONE) {
        target = Some(((r, w - 1), false));
    }
    if key(Key::PageDown, Modifiers::NONE) {
        target = Some(((next_row(st, r, page), c), false));
    }
    if key(Key::PageUp, Modifiers::NONE) {
        target = Some(((next_row(st, r, -page), c), false));
    }
    if let Some(((nr, nc), extend)) = target {
        st.select(nr, nc, extend);
    }
    if key(Key::F2, Modifiers::NONE) {
        start_edit(app, st, r, c, None);
        return;
    }
    if key(Key::Delete, Modifiers::NONE) {
        super::clear_selection(app, st);
    }
    if key(Key::Backspace, Modifiers::NONE) {
        super::clear_selection(app, st);
        start_edit(app, st, r, c, Some(String::new()));
        return;
    }
    if key(Key::D, Modifiers::COMMAND) {
        let sel = st.selection();
        let sel = if sel.rows() == 1 && sel.r1 > 0 { Range { r1: sel.r1 - 1, ..sel } } else { sel };
        super::apply(app, st, "Fill down", |s| ops::fill_down(s, sel));
    }
    if key(Key::A, Modifiers::COMMAND) {
        st.anchor = (0, 0);
        st.cursor = (last, w - 1);
    }
    if key(Key::F, Modifiers::COMMAND) {
        st.find.open = true;
        st.find.replace_mode = false;
    }
    if key(Key::H, Modifiers::COMMAND) {
        st.find.open = true;
        st.find.replace_mode = true;
    }
    if key(Key::Escape, Modifiers::NONE) {
        st.clip = None;
    }
}

// =============================================================================================
// Context menus
// =============================================================================================

fn cell_menu(app: &mut VizApp, st: &mut SheetUi, ui: &mut Ui) {
    let sel = st.selection();
    if ui.button("Cut").clicked() {
        super::copy(app, st, ui.ctx(), true);
        ui.close();
    }
    if ui.button("Copy").clicked() {
        super::copy(app, st, ui.ctx(), false);
        ui.close();
    }
    if ui.button("Paste").clicked() {
        if let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
            super::paste_text(app, st, &text);
        } else if let Some((tsv, _, _)) = st.clip.clone() {
            super::paste_text(app, st, &tsv);
        }
        ui.close();
    }
    ui.separator();
    if ui.button("Insert rows above").clicked() {
        apply_rows(app, st, true, sel.r1.max(1), sel.rows());
        ui.close();
    }
    if ui.button("Insert columns left").clicked() {
        let n = sel.cols();
        super::apply(app, st, "Insert columns", |s| ops::insert_cols(s, sel.c1, n));
        ui.close();
    }
    if ui.button("Delete rows").clicked() {
        apply_rows(app, st, false, sel.r1.max(1), sel.r2.max(1) - sel.r1.max(1) + 1);
        ui.close();
    }
    if ui.button("Delete columns").clicked() {
        let n = sel.cols();
        super::apply(app, st, "Delete columns", |s| ops::delete_cols(s, sel.c1, n));
        ui.close();
    }
    if ui.button("Clear contents").clicked() {
        super::clear_selection(app, st);
        ui.close();
    }
    ui.separator();
    if ui.button("Sort A to Z").clicked() {
        super::sort_by(app, st, sel.c1, false);
        ui.close();
    }
    if ui.button("Sort Z to A").clicked() {
        super::sort_by(app, st, sel.c1, true);
        ui.close();
    }
    if ui.button("Filter by this value").clicked() {
        filter_by_value(app, st);
        ui.close();
    }
    if ui.button("Fill down").clicked() {
        let sel = if sel.rows() == 1 && sel.r1 > 0 { Range { r1: sel.r1 - 1, ..sel } } else { sel };
        super::apply(app, st, "Fill down", |s| ops::fill_down(s, sel));
        ui.close();
    }
}

fn apply_rows(app: &mut VizApp, st: &mut SheetUi, insert: bool, at: usize, n: usize) {
    super::apply(app, st, if insert { "Insert rows" } else { "Delete rows" }, |s| if insert { ops::insert_rows(s, at, n) } else { ops::delete_rows(s, at, n) });
}

fn row_menu(app: &mut VizApp, st: &mut SheetUi, ui: &mut Ui) {
    let sel = st.selection();
    let (r1, n) = (sel.r1.max(1), sel.r2.max(1) - sel.r1.max(1) + 1);
    if ui.button(format!("Insert {n} row{} above", if n == 1 { "" } else { "s" })).clicked() {
        apply_rows(app, st, true, r1, n);
        ui.close();
    }
    if ui.button(format!("Insert {n} row{} below", if n == 1 { "" } else { "s" })).clicked() {
        apply_rows(app, st, true, sel.r2 + 1, n);
        ui.close();
    }
    if ui.button(format!("Delete {n} row{}", if n == 1 { "" } else { "s" })).clicked() {
        apply_rows(app, st, false, r1, n);
        ui.close();
    }
    if ui.button("Clear contents").clicked() {
        super::clear_selection(app, st);
        ui.close();
    }
}

fn col_menu(app: &mut VizApp, st: &mut SheetUi, ui: &mut Ui, c: usize) {
    let sel = st.selection();
    let (c1, n) = if c >= sel.c1 && c <= sel.c2 { (sel.c1, sel.cols()) } else { (c, 1) };
    if ui.button("Sort A to Z").clicked() {
        super::sort_by(app, st, c, false);
        ui.close();
    }
    if ui.button("Sort Z to A").clicked() {
        super::sort_by(app, st, c, true);
        ui.close();
    }
    ui.separator();
    if ui.button("Insert column left").clicked() {
        super::apply(app, st, "Insert columns", |s| ops::insert_cols(s, c1, n));
        ui.close();
    }
    if ui.button("Insert column right").clicked() {
        super::apply(app, st, "Insert columns", |s| ops::insert_cols(s, c1 + n, n));
        ui.close();
    }
    if ui.button(format!("Delete column{}", if n == 1 { "" } else { "s" })).clicked() {
        super::apply(app, st, "Delete columns", |s| ops::delete_cols(s, c1, n));
        ui.close();
    }
    if ui.button("Autofit width").clicked() {
        super::apply(app, st, "Column width", |s| {
            if let Some(w) = s.col_widths.get_mut(c) {
                *w = 0.0;
            }
        });
        ui.close();
    }
    ui.separator();
    ui.menu_button("Number format", |ui| {
        for (f, label) in super::FORMATS {
            if ui.button(*label).clicked() {
                st.anchor = (0, c);
                st.cursor = (0, c);
                super::set_col_format(app, st, f);
                ui.close();
            }
        }
    });
    ui.menu_button("Data type", |ui| {
        for (ty, label) in super::TYPES {
            if ui.button(*label).clicked() {
                st.anchor = (0, c);
                st.cursor = (0, c);
                super::set_col_type(app, st, ty);
                ui.close();
            }
        }
    });
}

fn filter_by_value(app: &VizApp, st: &mut SheetUi) {
    let Some(s) = app.project.sheets.get(st.active) else { return };
    let Some(cache) = &st.cache else { return };
    let (r, c) = st.cursor;
    let keep = super::display_value(s, &cache.values, r, c);
    let mut hidden = std::collections::BTreeSet::new();
    for row in 1..s.cells.len() {
        let v = super::display_value(s, &cache.values, row, c);
        if v != keep {
            hidden.insert(v);
        }
    }
    st.filters.entry(s.name.clone()).or_default().insert(c, hidden);
}

// =============================================================================================
// Filter popup (value checklist + sort)
// =============================================================================================

pub fn filter_popup(app: &mut VizApp, st: &mut SheetUi, ctx: &egui::Context) {
    let Some(mut fp) = st.filter_popup.take() else { return };
    let Some(s) = app.project.sheets.get(st.active) else { return };
    let Some(cache) = &st.cache else {
        st.filter_popup = Some(fp);
        return;
    };
    let c = fp.col;
    if fp.values.is_empty() {
        let hidden = st.filters.get(&s.name).and_then(|f| f.get(&c)).cloned().unwrap_or_default();
        let mut vals: Vec<(String, CellValue)> = (1..s.cells.len()).map(|r| (super::display_value(s, &cache.values, r, c), cache.values.get(r).and_then(|x| x.get(c)).cloned().unwrap_or_default())).collect();
        vals.sort_by(|a, b| vizcraft_core::scalar::compare(&a.1.to_scalar(), &b.1.to_scalar()));
        vals.dedup_by(|a, b| a.0 == b.0);
        fp.values = vals.into_iter().map(|(v, _)| {
            let shown = !hidden.contains(&v);
            (v, shown)
        }).collect();
    }
    let t = crate::theme::tokens(ctx);
    let title = ops::get(s, 0, c).to_string();
    let mut close = false;
    let mut apply_now = false;
    let mut sort: Option<bool> = None;
    let area = egui::Area::new(egui::Id::unique("sheet_filter_popup")).fixed_pos(Pos2::new(fp.pos.x.max(4.0), fp.pos.y)).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(230.0);
            ui.label(egui::RichText::new(&title).strong());
            ui.add_space(2.0);
            if ui.button("Sort A to Z").clicked() {
                sort = Some(false);
            }
            if ui.button("Sort Z to A").clicked() {
                sort = Some(true);
            }
            ui.separator();
            ui.add(egui::TextEdit::singleline(&mut fp.search).hint_text("Search values").desired_width(f32::INFINITY));
            let needle = fp.search.to_lowercase();
            let all = fp.values.iter().filter(|v| needle.is_empty() || v.0.to_lowercase().contains(&needle)).all(|v| v.1);
            let mut all2 = all;
            if ui.checkbox(&mut all2, egui::RichText::new("(Select all)").italics()).changed() {
                for v in fp.values.iter_mut() {
                    if needle.is_empty() || v.0.to_lowercase().contains(&needle) {
                        v.1 = all2;
                    }
                }
            }
            egui::ScrollArea::vertical().max_height(240.0).auto_shrink([false, true]).show(ui, |ui| {
                let mut shown = 0;
                for v in fp.values.iter_mut() {
                    if !needle.is_empty() && !v.0.to_lowercase().contains(&needle) {
                        continue;
                    }
                    shown += 1;
                    if shown > 1000 {
                        ui.label(egui::RichText::new("… search to narrow the list").color(t.muted));
                        break;
                    }
                    let label = if v.0.is_empty() { "(Blanks)".to_string() } else { v.0.clone() };
                    ui.checkbox(&mut v.1, label);
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(egui::RichText::new("Apply").color(Color32::WHITE)).fill(t.accent)).clicked() {
                    apply_now = true;
                }
                if ui.button("Clear filter").clicked() {
                    for v in fp.values.iter_mut() {
                        v.1 = true;
                    }
                    apply_now = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
    });
    let name = s.name.clone();
    if let Some(desc) = sort {
        super::sort_by(app, st, c, desc);
        return;
    }
    if apply_now {
        let hidden: std::collections::BTreeSet<String> = fp.values.iter().filter(|v| !v.1).map(|v| v.0.clone()).collect();
        let f = st.filters.entry(name).or_default();
        if hidden.is_empty() {
            f.remove(&c);
        } else {
            f.insert(c, hidden);
        }
        return;
    }
    let clicked_outside = ctx.input(|i| i.pointer.any_pressed()) && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|p| !area.response.rect.contains(p));
    if close || clicked_outside || ctx.input(|i| i.key_pressed(Key::Escape)) {
        return;
    }
    st.filter_popup = Some(fp);
}
