//! The flow canvas: nodes as cards with ports, bezier wires, pan / zoom, drag to connect,
//! drop from the palette.

use std::collections::HashMap;

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use vizcraft_core::doc::{Flow, FlowNode};
use vizcraft_core::flow::{self, NodeType, PortKind, Status};

use super::{FlowsUi, View, category_color};
use crate::app::VizApp;
use crate::theme::Tokens;

pub const NODE_W: f32 = 168.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Connect {
    pub node: u64,
    pub port: usize,
    pub output: bool,
    pub kind: PortKind,
}

pub fn node_h(nt: Option<&NodeType>) -> f32 {
    let ports = nt.map(|n| n.inputs.len().max(n.outputs.len())).unwrap_or(1);
    (32.0 + 18.0 * ports as f32).max(62.0)
}

fn port_pos(n: &FlowNode, nt: Option<&NodeType>, idx: usize, output: bool) -> Pos2 {
    let h = node_h(nt);
    let count = nt.map(|t| if output { t.outputs.len() } else { t.inputs.len() }).unwrap_or(1).max(1);
    let y = n.y + h * (idx + 1) as f32 / (count + 1) as f32;
    pos2(if output { n.x + NODE_W } else { n.x }, y)
}

fn port_color(k: PortKind) -> Color32 {
    match k {
        PortKind::Table => Color32::from_rgb(0x2B, 0x6C, 0xE0),
        PortKind::Model => Color32::from_rgb(0x8B, 0x5C, 0xF6),
    }
}

fn wire_color(t: &Tokens) -> Color32 {
    if t.dark { Color32::from_rgb(0x6B, 0x76, 0x8A) } else { Color32::from_rgb(0x94, 0xA3, 0xB8) }
}

fn bezier(a: Pos2, b: Pos2) -> [Pos2; 4] {
    let dx = ((b.x - a.x).abs() * 0.5).max(40.0);
    [a, a + vec2(dx, 0.0), b - vec2(dx, 0.0), b]
}

fn bezier_point(p: &[Pos2; 4], t: f32) -> Pos2 {
    let u = 1.0 - t;
    let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
    pos2(p[0].x * w[0] + p[1].x * w[1] + p[2].x * w[2] + p[3].x * w[3], p[0].y * w[0] + p[1].y * w[1] + p[2].y * w[2] + p[3].y * w[3])
}

pub fn show(app: &mut VizApp, st: &mut FlowsUi, ui: &mut Ui, t: &Tokens, fi: usize, statuses: &HashMap<u64, Status>) {
    let Some(f) = app.project.flows.get(fi) else { return };
    let fname = f.name.clone();
    let rect = ui.available_rect_before_wrap();
    let bg = ui.allocate_rect(rect, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let types: HashMap<u64, Option<NodeType>> = f.nodes.iter().map(|n| (n.id, flow::node_type(&n.kind))).collect();
    // Fit.
    if st.fit_request {
        st.fit_request = false;
        if !f.nodes.is_empty() {
            let mut b = Rect::NOTHING;
            for n in &f.nodes {
                b = b.union(Rect::from_min_size(pos2(n.x, n.y), vec2(NODE_W, node_h(types.get(&n.id).and_then(|x| x.as_ref())))));
            }
            let zoom = ((rect.width() - 120.0) / b.width()).min((rect.height() - 100.0) / b.height()).clamp(0.5, 1.15);
            let pan = rect.size() / 2.0 - b.center().to_vec2() * zoom;
            st.views.insert(fname.clone(), View { pan, zoom });
        }
    }
    let mut view = st.views.get(&fname).copied().unwrap_or_default();
    // Pan / zoom input.
    if bg.hovered() || ui.rect_contains_pointer(rect) {
        let (scroll, zoom_d, ptr) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.pointer.hover_pos()));
        if zoom_d != 1.0
            && let Some(p) = ptr
        {
            let old = view.zoom;
            view.zoom = (view.zoom * zoom_d).clamp(0.3, 2.5);
            let local = p - rect.min - view.pan;
            view.pan += local - local * (view.zoom / old);
        } else if scroll != egui::Vec2::ZERO && st.palette_drag.is_none() {
            view.pan += scroll;
        }
    }
    if bg.dragged() && st.connecting.is_none() && st.dragging_node.is_none() {
        view.pan += bg.drag_delta();
    }
    if ui.input(|i| i.pointer.middle_down()) && ui.rect_contains_pointer(rect) {
        view.pan += ui.input(|i| i.pointer.delta());
    }
    if bg.clicked() {
        st.selected = None;
    }
    let z = view.zoom;
    let to_s = |p: Pos2| rect.min + view.pan + p.to_vec2() * z;
    let to_c = |p: Pos2| ((p - rect.min - view.pan) / z).to_pos2();

    // Dot grid.
    let step = 24.0 * z;
    if step > 6.0 {
        let ox = (rect.left() + view.pan.x).rem_euclid(step);
        let oy = (rect.top() + view.pan.y).rem_euclid(step);
        let dot = if t.dark { Color32::from_rgb(0x2A, 0x30, 0x3C) } else { Color32::from_rgb(0xCF, 0xD5, 0xDE) };
        let mut y = rect.top() + oy;
        while y < rect.bottom() {
            let mut x = rect.left() + ox;
            while x < rect.right() {
                painter.circle_filled(pos2(x, y), 1.0, dot);
                x += step;
            }
            y += step;
        }
    }
    if f.nodes.is_empty() {
        painter.text(rect.center(), Align2::CENTER_CENTER, "Drag nodes from the palette to start (or click one to add it)", FontId::proportional(14.0), t.muted);
    }

    // Wires.
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let mut delete_link: Option<usize> = None;
    let sel = st.selected;
    for (li, l) in f.links.iter().enumerate() {
        let (Some(a), Some(b)) = (f.nodes.iter().find(|n| n.id == l.0), f.nodes.iter().find(|n| n.id == l.2)) else { continue };
        let ta = types.get(&a.id).and_then(|x| x.as_ref());
        let tb = types.get(&b.id).and_then(|x| x.as_ref());
        let kind = ta.and_then(|x| x.outputs.get(l.1)).map(|p| p.kind).unwrap_or(PortKind::Table);
        let pts = bezier(to_s(port_pos(a, ta, l.1, true)), to_s(port_pos(b, tb, l.3, false)));
        let hl = sel == Some(a.id) || sel == Some(b.id);
        let near = pointer.is_some_and(|p| (0..=24).any(|k| (bezier_point(&pts, k as f32 / 24.0) - p).length() < 6.0)) && st.connecting.is_none() && st.dragging_node.is_none();
        let col = if near { t.danger } else if hl { t.accent } else if kind == PortKind::Model { port_color(PortKind::Model).gamma_multiply(0.8) } else { wire_color(t) };
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(pts, false, Color32::TRANSPARENT, Stroke::new(if hl || near { 2.6 } else { 2.0 } * z.max(0.7), col)));
        if near {
            let mid = bezier_point(&pts, 0.5);
            let r = Rect::from_center_size(mid, vec2(18.0, 18.0));
            painter.circle(mid, 9.0, t.panel, Stroke::new(1.5, t.danger));
            painter.line_segment([mid - vec2(3.5, 3.5), mid + vec2(3.5, 3.5)], Stroke::new(1.6, t.danger));
            painter.line_segment([mid - vec2(3.5, -3.5), mid + vec2(3.5, -3.5)], Stroke::new(1.6, t.danger));
            let resp = ui.interact(r, ui.scope_id().with(("wire", li)), Sense::click()).on_hover_text("Remove this connection");
            if resp.clicked() {
                delete_link = Some(li);
            }
        }
    }

    // Nodes.
    let mut select: Option<u64> = None;
    let mut drag_delta: Option<(u64, egui::Vec2)> = None;
    let mut drag_start: Option<u64> = None;
    let mut drag_stop = false;
    let mut run_node: Option<u64> = None;
    let mut menu_action: Option<(&str, u64)> = None;
    let mut port_press: Option<Connect> = None;
    let mut port_hits: Vec<(Rect, Connect)> = Vec::new();
    for n in &f.nodes {
        let nt = types.get(&n.id).and_then(|x| x.as_ref());
        let h = node_h(nt);
        let r = Rect::from_min_size(to_s(pos2(n.x, n.y)), vec2(NODE_W, h) * z);
        if !r.expand(40.0).intersects(rect) {
            // Still register ports for hit testing (off-screen ports can't be hit anyway).
            continue;
        }
        let cat = nt.map(|x| x.category).unwrap_or("Output");
        let col = category_color(cat);
        let selected = sel == Some(n.id);
        let status = statuses.get(&n.id).copied().unwrap_or(Status::Stale);
        // Shadow + body.
        painter.rect_filled(r.translate(vec2(0.0, 2.0 * z)), 10.0 * z, Color32::from_black_alpha(if t.dark { 60 } else { 18 }));
        painter.rect(r, 10.0 * z, t.panel, Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { t.accent } else { t.border }), StrokeKind::Inside);
        // Icon tile.
        let tile = Rect::from_min_size(r.min + vec2(10.0, 10.0) * z, vec2(32.0, 32.0) * z);
        painter.rect_filled(tile, 8.0 * z, col);
        super::glyphs::paint(&painter, tile.shrink(6.0 * z), nt.map(|x| x.icon).unwrap_or("table"), Color32::WHITE);
        // Texts.
        let text_clip = painter.with_clip_rect(Rect::from_min_max(r.min + vec2(48.0, 4.0) * z, r.max - vec2(18.0, 4.0) * z).intersect(rect));
        let text_w = (NODE_W - 66.0) * z;
        let fs1 = (13.0 * z).max(9.5);
        let fs2 = (11.0 * z).max(8.0);
        text_clip.text(r.min + vec2(50.0, 17.0) * z, Align2::LEFT_CENTER, vizcraft_core::scene::ellipsize(&n.label, fs1, text_w), FontId::proportional(fs1), t.text);
        let cached = st.executor.cached(&f.name, n.id);
        let sub = match (status, cached) {
            (Status::Error, _) => "Error".to_string(),
            (_, Some(Ok(o))) => match o.table(0) {
                Some(tb) => format!("{} rows · {} cols", vizcraft_core::numfmt::format(tb.rows.len() as f64, "#,0"), tb.columns.len()),
                None => nt.map(|x| x.label.to_string()).unwrap_or_default(),
            },
            _ => nt.map(|x| x.label.to_string()).unwrap_or_else(|| n.kind.clone()),
        };
        text_clip.text(r.min + vec2(50.0, 35.0) * z, Align2::LEFT_CENTER, vizcraft_core::scene::ellipsize(&sub, fs2, text_w), FontId::proportional(fs2), if status == Status::Error { t.danger } else { t.muted });
        if nt.is_some_and(|x| x.label != n.label) && h > 64.0 && z >= 0.8 {
            text_clip.text(r.min + vec2(50.0, 51.0) * z, Align2::LEFT_CENTER, nt.map(|x| x.label).unwrap_or(""), FontId::proportional((10.0 * z).max(5.0)), col);
        }
        // Status dot.
        let dc = match status {
            Status::Ok => t.good,
            Status::Error => t.danger,
            Status::Stale => Color32::from_rgb(0xA0, 0xA8, 0xB8),
        };
        let dot = pos2(r.right() - 11.0 * z, r.top() + 11.0 * z);
        painter.circle(dot, 4.5 * z, dc, Stroke::new(1.5 * z, t.panel));
        if super::in_model(app, fi, n.id).is_some() {
            let b = Rect::from_min_size(pos2(r.right() - 46.0 * z, r.bottom() - 18.0 * z), vec2(38.0, 13.0) * z);
            painter.rect_filled(b, 6.0 * z, t.good.gamma_multiply(0.15));
            painter.text(b.center(), Align2::CENTER_CENTER, "model", FontId::proportional((9.0 * z).max(5.0)), t.good);
        }
        // Body interaction.
        let resp = ui.interact(r, ui.scope_id().with(("node", n.id)), Sense::click_and_drag());
        let resp = if status == Status::Error {
            match cached {
                Some(Err(e)) => resp.on_hover_text(e.clone()),
                _ => resp,
            }
        } else {
            resp.on_hover_text(nt.map(|x| x.help).unwrap_or(""))
        };
        if resp.clicked() || resp.drag_started() {
            select = Some(n.id);
        }
        if resp.double_clicked() {
            run_node = Some(n.id);
        }
        if resp.drag_started() {
            drag_start = Some(n.id);
        }
        if resp.dragged() {
            drag_delta = Some((n.id, resp.drag_delta() / z));
        }
        if resp.drag_stopped() {
            drag_stop = true;
        }
        resp.context_menu(|ui| {
            if ui.button("Run").clicked() {
                menu_action = Some(("run", n.id));
                ui.close();
            }
            if nt.is_some_and(|x| x.outputs.first().is_some_and(|p| p.kind == PortKind::Table)) && ui.button("Add to model").clicked() {
                menu_action = Some(("model", n.id));
                ui.close();
            }
            if ui.button("Duplicate").clicked() {
                menu_action = Some(("dup", n.id));
                ui.close();
            }
            if ui.button("Disconnect").clicked() {
                menu_action = Some(("disconnect", n.id));
                ui.close();
            }
            ui.separator();
            if ui.button(egui::RichText::new("Delete").color(t.danger)).clicked() {
                menu_action = Some(("delete", n.id));
                ui.close();
            }
        });
        // Ports (after the body so they take the pointer).
        if let Some(nt) = nt {
            for (output, ports) in [(false, &nt.inputs), (true, &nt.outputs)] {
                for (i, p) in ports.iter().enumerate() {
                    let c = to_s(port_pos(n, Some(nt), i, output));
                    let pc = port_color(p.kind);
                    let connected = f.links.iter().any(|l| if output { l.0 == n.id && l.1 == i } else { l.2 == n.id && l.3 == i });
                    let cc = Connect { node: n.id, port: i, output, kind: p.kind };
                    let compatible = st.connecting.is_some_and(|k| k.output != output && k.kind == p.kind && k.node != n.id);
                    let hit = Rect::from_center_size(c, vec2(16.0, 16.0) * z.max(0.8));
                    let pr = ui.interact(hit, ui.scope_id().with(("port", n.id, output, i)), Sense::click_and_drag());
                    let hot = pr.hovered() || (compatible && pointer.is_some_and(|q| hit.expand(6.0).contains(q)));
                    let rad = if hot { 7.0 } else { 5.5 } * z.max(0.7);
                    if p.kind == PortKind::Model {
                        let d = Rect::from_center_size(c, vec2(rad * 2.0, rad * 2.0));
                        painter.add(egui::Shape::convex_polygon(vec![d.center_top(), d.right_center(), d.center_bottom(), d.left_center()], if connected || hot { pc } else { t.panel }, Stroke::new(2.0, pc)));
                    } else {
                        painter.circle(c, rad, if connected || hot { pc } else { t.panel }, Stroke::new(2.0, if compatible { t.good } else { pc }));
                    }
                    let show_labels = selected || resp.hovered() || st.connecting.is_some();
                    if show_labels && (ports.len() > 1 || p.kind == PortKind::Model || p.optional) {
                        let lbl = if p.optional { format!("{} (optional)", p.name) } else { p.name.to_string() };
                        let (pos, al) = if output { (c + vec2(10.0 * z, -9.0 * z), Align2::LEFT_CENTER) } else { (c + vec2(-10.0 * z, -9.0 * z), Align2::RIGHT_CENTER) };
                        if z > 0.6 {
                            painter.text(pos, al, lbl, FontId::proportional(9.5 * z), t.muted);
                        }
                    }
                    let pr = pr.on_hover_text(format!("{} {} ({})", if output { "Output" } else { "Input" }, p.name, if p.kind == PortKind::Model { "model" } else { "table" }));
                    if pr.drag_started() {
                        port_press = Some(cc);
                    }
                    port_hits.push((hit.expand(6.0), cc));
                }
            }
        }
    }
    // Apply node interactions.
    if let Some(id) = select {
        st.selected = Some(id);
        st.preview_port = 0;
    }
    if let Some(id) = drag_start {
        super::checkpoint_grouped(app, st, &format!("move{id}"), "Move node");
        st.dragging_node = Some(id);
    }
    if let Some((id, d)) = drag_delta
        && let Some(n) = app.project.flows.get_mut(fi).and_then(|f| f.nodes.iter_mut().find(|n| n.id == id))
    {
        n.x += d.x;
        n.y += d.y;
    }
    if drag_stop {
        st.dragging_node = None;
        // Snap to the grid.
        if let Some(id) = st.selected
            && let Some(n) = app.project.flows.get_mut(fi).and_then(|f| f.nodes.iter_mut().find(|n| n.id == id))
        {
            n.x = (n.x / 4.0).round() * 4.0;
            n.y = (n.y / 4.0).round() * 4.0;
        }
    }
    if let Some(id) = run_node {
        st.run_request = Some(super::RunRequest::Node(id));
    }
    if let Some(li) = delete_link {
        app.checkpoint("Disconnect");
        if let Some(f) = app.project.flows.get_mut(fi)
            && li < f.links.len()
        {
            f.links.remove(li);
        }
    }
    match menu_action {
        Some(("run", id)) => st.run_request = Some(super::RunRequest::Node(id)),
        Some(("model", id)) => super::add_to_model(app, fi, id),
        Some(("dup", id)) => {
            if let Some(mut n) = app.project.flows.get(fi).and_then(|f| f.nodes.iter().find(|n| n.id == id)).cloned() {
                app.checkpoint("Duplicate node");
                n.id = app.project.next_id();
                n.x += 30.0;
                n.y += 30.0;
                let nid = n.id;
                if let Some(f) = app.project.flows.get_mut(fi) {
                    f.nodes.push(n);
                }
                st.selected = Some(nid);
            }
        }
        Some(("disconnect", id)) => {
            app.checkpoint("Disconnect node");
            if let Some(f) = app.project.flows.get_mut(fi) {
                f.links.retain(|l| l.0 != id && l.2 != id);
            }
        }
        Some(("delete", id)) => delete_node(app, st, fi, id),
        _ => {}
    }
    // Start connecting (dragging from an input with a link picks that link up).
    if let Some(c) = port_press {
        if !c.output
            && let Some(f) = app.project.flows.get(fi)
            && let Some(li) = f.links.iter().position(|l| l.2 == c.node && l.3 == c.port)
        {
            let l = f.links.get(li).copied();
            app.checkpoint("Reconnect");
            if let Some(f) = app.project.flows.get_mut(fi) {
                f.links.remove(li);
            }
            if let Some(l) = l {
                st.connecting = Some(Connect { node: l.0, port: l.1, output: true, kind: c.kind });
            }
        } else {
            st.connecting = Some(c);
        }
    }
    // Connecting wire + drop.
    if let Some(c) = st.connecting {
        let Some(f) = app.project.flows.get(fi) else { return };
        if let (Some(n), Some(p)) = (f.nodes.iter().find(|n| n.id == c.node), pointer) {
            let nt = types.get(&n.id).and_then(|x| x.as_ref());
            let a = to_s(port_pos(n, nt, c.port, c.output));
            let pts = if c.output { bezier(a, p) } else { bezier(p, a) };
            painter.add(egui::epaint::CubicBezierShape::from_points_stroke(pts, false, Color32::TRANSPARENT, Stroke::new(2.2, t.accent)));
        }
        if ui.input(|i| i.pointer.any_released()) {
            let target = pointer.and_then(|p| port_hits.iter().find(|(r, k)| r.contains(p) && k.output != c.output && k.kind == c.kind && k.node != c.node)).map(|x| x.1);
            if let Some(k) = target {
                let (from, to) = if c.output { (c, k) } else { (k, c) };
                connect(app, fi, from.node, from.port, to.node, to.port);
            }
            st.connecting = None;
        }
    }
    // Drop from the palette.
    if let Some(kind) = st.palette_drag.clone() {
        if let Some(p) = pointer {
            if let Some(nt) = flow::node_type(&kind) {
                let ghost = Rect::from_min_size(p - vec2(20.0, 16.0), vec2(NODE_W, 50.0) * z.max(0.6));
                let lp = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, ui.scope_id().with("ghost")));
                lp.rect(ghost, 10.0, t.panel.gamma_multiply(0.92), Stroke::new(1.5, t.accent), StrokeKind::Inside);
                let tile = Rect::from_min_size(ghost.min + vec2(9.0, 9.0), vec2(30.0, 30.0));
                lp.rect_filled(tile, 7.0, category_color(nt.category));
                super::glyphs::paint(&lp, tile.shrink(6.0), nt.icon, Color32::WHITE);
                lp.text(ghost.min + vec2(48.0, 24.0), Align2::LEFT_CENTER, nt.label, FontId::proportional(12.5), t.text);
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if ui.input(|i| i.pointer.any_released()) {
            if let Some(p) = pointer.filter(|p| rect.contains(*p)) {
                let at = to_c(p - vec2(20.0, 16.0));
                add_node(app, st, fi, &kind, at.x, at.y, None);
            }
            st.palette_drag = None;
        }
    }
    // Keyboard.
    if let Some(id) = st.selected
        && !ui.ctx().text_edit_focused()
        && ui.input(|i| i.key_pressed(egui::Key::Delete))
        && ui.rect_contains_pointer(rect)
    {
        delete_node(app, st, fi, id);
    }
    st.views.insert(fname, view);
}

/// Links an output to an input (an input takes one link: the old one is replaced).
pub fn connect(app: &mut VizApp, fi: usize, from: u64, fport: usize, to: u64, tport: usize) {
    let Some(f) = app.project.flows.get(fi) else { return };
    if creates_cycle(f, from, to) {
        app.error("That connection would create a loop.");
        return;
    }
    app.checkpoint("Connect");
    if let Some(f) = app.project.flows.get_mut(fi) {
        f.links.retain(|l| !(l.2 == to && l.3 == tport));
        f.links.push((from, fport, to, tport));
    }
}

fn creates_cycle(f: &Flow, from: u64, to: u64) -> bool {
    // Is `from` reachable from `to`?
    let mut stack = vec![to];
    let mut seen = std::collections::HashSet::new();
    while let Some(n) = stack.pop() {
        if n == from {
            return true;
        }
        if seen.insert(n) {
            stack.extend(f.links.iter().filter(|l| l.0 == n).map(|l| l.2));
        }
    }
    false
}

pub fn delete_node(app: &mut VizApp, st: &mut FlowsUi, fi: usize, id: u64) {
    let label = app.project.flows.get(fi).and_then(|f| f.nodes.iter().find(|n| n.id == id)).map(|n| n.label.clone()).unwrap_or_default();
    app.checkpoint("Delete node");
    if let Some(f) = app.project.flows.get_mut(fi) {
        f.nodes.retain(|n| n.id != id);
        f.links.retain(|l| l.0 != id && l.2 != id);
    }
    if st.selected == Some(id) {
        st.selected = None;
    }
    if super::in_model(app, fi, id).is_some() {
        app.error(format!("“{label}” fed a model table: that table will fail to refresh until you remove it."));
    }
}

/// Default settings for a new node (sources pick the first available source).
fn initial_config(app: &VizApp, kind: &str) -> std::collections::BTreeMap<String, serde_json::Value> {
    let mut m = std::collections::BTreeMap::new();
    match kind {
        "source.sheet" => {
            if let Some(s) = app.project.sheets.first() {
                m.insert("name".into(), serde_json::json!(s.name));
            }
        }
        "source.query" => {
            if let Some(q) = app.project.queries.queries.first() {
                m.insert("name".into(), serde_json::json!(q.name));
            }
        }
        "source.table" => {
            if let Some(t) = app.project.model.tables.first() {
                m.insert("name".into(), serde_json::json!(t.name));
            }
        }
        _ => {}
    }
    m
}

pub fn add_node(app: &mut VizApp, st: &mut FlowsUi, fi: usize, kind: &str, x: f32, y: f32, link_from: Option<(u64, usize)>) -> Option<u64> {
    let nt = flow::node_type(kind)?;
    app.checkpoint("Add node");
    let id = app.project.next_id();
    let mut config = initial_config(app, kind);
    let label = match config.get("name") {
        Some(serde_json::Value::String(s)) if kind.starts_with("source.") => s.clone(),
        _ => nt.label.to_string(),
    };
    if kind == "out.table" {
        config.insert("name".into(), serde_json::json!(app.project.free_table_name("Flow output")));
    }
    let node = FlowNode { id, kind: kind.into(), label, x: (x / 4.0).round() * 4.0, y: (y / 4.0).round() * 4.0, config };
    if let Some(f) = app.project.flows.get_mut(fi) {
        f.nodes.push(node);
        if let Some((from, port)) = link_from
            && nt.inputs.first().is_some_and(|p| p.kind == PortKind::Table)
        {
            f.links.retain(|l| !(l.2 == id && l.3 == 0));
            f.links.push((from, port, id, 0));
        }
    }
    st.selected = Some(id);
    st.preview_port = 0;
    super::checkpoint_grouped(app, st, "add", "Add node");
    Some(id)
}

/// Palette click: adds a node right of the selected one and links it.
pub fn add_node_after_selection(app: &mut VizApp, st: &mut FlowsUi, fi: usize, kind: &str) {
    let Some(f) = app.project.flows.get(fi) else { return };
    let sel = st.selected.and_then(|id| f.nodes.iter().find(|n| n.id == id)).cloned();
    let fname = f.name.clone();
    let (mut x, mut y, link) = match &sel {
        Some(n) => {
            let out_table = flow::node_type(&n.kind).is_some_and(|t| t.outputs.first().is_some_and(|p| p.kind == PortKind::Table));
            (n.x + NODE_W + 60.0, n.y, if out_table { Some((n.id, 0)) } else { None })
        }
        None => {
            let v = st.views.get(&fname).copied().unwrap_or_default();
            (((-v.pan.x) + 300.0) / v.zoom, ((-v.pan.y) + 160.0) / v.zoom, None)
        }
    };
    // Avoid stacking on another node.
    for _ in 0..20 {
        if f.nodes.iter().any(|n| (n.x - x).abs() < 60.0 && (n.y - y).abs() < 40.0) {
            y += 90.0;
        } else {
            break;
        }
    }
    if !x.is_finite() || !y.is_finite() {
        x = 60.0;
        y = 60.0;
    }
    add_node(app, st, fi, kind, x, y, link);
}
