//! The data source canvas (Tableau style): drag tables from the list onto the canvas. The first
//! is the root; a table dropped beside another joins it (a Venn icon on the link: click it to
//! pick the join kind and the clauses); a table dropped on the lower half of another unions
//! with it. The bottom shows the combined result live. Saving makes a Power Query query whose M
//! uses `Table.NestedJoin` / `Table.ExpandTableColumn` / `Table.Combine`, so joins and unions
//! end up in Power Query like any other step.

use std::collections::HashMap;
use std::rc::Rc;

use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use vizcraft_query::steps::{Step, build_m, ident, text_list};
use vizcraft_query::value::quote_text;
use vizcraft_query::{Table, Value};

use super::forms::{self, primary};
use crate::app::VizApp;
use crate::theme::Tokens;

const JOINS: &[(&str, &str)] = &[("Inner", "Inner"), ("Left", "LeftOuter"), ("Right", "RightOuter"), ("Full Outer", "FullOuter"), ("Left Anti", "LeftAnti"), ("Right Anti", "RightAnti")];
const BOX_W: f32 = 180.0;
const BOX_H: f32 = 38.0;
const MEMBER_H: f32 = 20.0;

/// A table on the canvas: one table, or several unioned.
#[derive(Clone, Debug, Default)]
pub struct Node {
    pub tables: Vec<String>,
    pub parent: Option<usize>,
    /// Index into the join kinds (Inner, Left, Right, Full Outer, Left Anti, Right Anti).
    pub join: usize,
    /// (parent column, this node's column) pairs.
    pub clauses: Vec<(String, String)>,
}

#[derive(Default)]
pub struct Canvas {
    pub nodes: Vec<Node>,
    pub name: String,
    pub search: String,
    pub drag: Option<String>,
    /// The node whose join is being edited.
    pub editing: Option<usize>,
    preview: Option<(String, Result<Rc<Table>, String>)>,
}

impl Node {
    fn label(&self) -> String {
        match self.tables.len() {
            0 => String::new(),
            1 => self.tables.first().cloned().unwrap_or_default(),
            n => format!("Union ({n})"),
        }
    }
    fn height(&self) -> f32 {
        BOX_H + if self.tables.len() > 1 { MEMBER_H * self.tables.len() as f32 + 4.0 } else { 0.0 }
    }
}

/// The columns of a node (the union of its tables' columns).
fn node_columns(app: &mut VizApp, n: &Node) -> Vec<String> {
    let mut cols: Vec<String> = Vec::new();
    for t in &n.tables {
        if let Ok(tb) = super::table_of(app, t) {
            for c in &tb.columns {
                if !cols.contains(c) {
                    cols.push(c.clone());
                }
            }
        }
    }
    cols
}

/// Guesses join clauses from column names both sides share (keys first).
fn guess_clauses(left: &[String], right: &[String]) -> Vec<(String, String)> {
    let common: Vec<&String> = left.iter().filter(|c| right.contains(c)).collect();
    let key = common.iter().find(|c| {
        let l = c.to_lowercase();
        l.ends_with("id") || l.ends_with("key") || l.ends_with("code")
    });
    match key.or(common.first()) {
        Some(c) => vec![((*c).clone(), (*c).clone())],
        None => vec![],
    }
}

/// The M of one node's table: the table itself, or `Table.Combine` of its members.
fn node_expr(n: &Node) -> String {
    match n.tables.len() {
        1 => ident(n.tables.first().map(String::as_str).unwrap_or_default()),
        _ => format!("Table.Combine({{{}}})", n.tables.iter().map(|t| ident(t)).collect::<Vec<_>>().join(", ")),
    }
}

fn dfs_order(nodes: &[Node]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut stack = vec![0usize];
    while let Some(i) = stack.pop() {
        if out.contains(&i) || i >= nodes.len() {
            continue;
        }
        out.push(i);
        let kids: Vec<usize> = nodes.iter().enumerate().filter(|(_, n)| n.parent == Some(i)).map(|(k, _)| k).rev().collect();
        stack.extend(kids);
    }
    out
}

/// The M of the whole canvas.
pub fn build(app: &mut VizApp, nodes: &[Node]) -> Option<String> {
    let root = nodes.first()?;
    if root.tables.is_empty() {
        return None;
    }
    let mut steps = vec![Step { name: "Source".into(), expr: node_expr(root) }];
    let mut present: Vec<String> = node_columns(app, root);
    // Per node: original column → name in the combined table.
    let mut names: HashMap<usize, HashMap<String, String>> = HashMap::new();
    names.insert(0, present.iter().map(|c| (c.clone(), c.clone())).collect());
    for i in dfs_order(nodes).into_iter().skip(1) {
        let Some(n) = nodes.get(i) else { continue };
        let Some(p) = n.parent else { continue };
        let cols = node_columns(app, n);
        let clauses: Vec<(String, String)> = n.clauses.iter().filter(|(a, b)| !a.is_empty() && !b.is_empty()).cloned().collect();
        if clauses.is_empty() {
            continue;
        }
        let pmap = names.get(&p).cloned().unwrap_or_default();
        let left: Vec<String> = clauses.iter().map(|(a, _)| pmap.get(a).cloned().unwrap_or_else(|| a.clone())).collect();
        let right: Vec<String> = clauses.iter().map(|(_, b)| b.clone()).collect();
        let label = n.label();
        let mut nested = label.clone();
        while present.contains(&nested) {
            nested.push('_');
        }
        let prev = steps.last().map(|s| ident(&s.name)).unwrap_or_default();
        let jk = JOINS.get(n.join).map(|j| j.1).unwrap_or("Inner");
        let merged = format!("Merged {label}");
        steps.push(Step { name: merged.clone(), expr: format!("Table.NestedJoin({prev}, {}, {}, {}, {}, JoinKind.{jk})", text_list(&left), node_expr(n), text_list(&right), quote_text(&nested)) });
        let mut map = HashMap::new();
        let mut new_names = Vec::new();
        for c in &cols {
            let mut nn = c.clone();
            if present.contains(&nn) || nn == nested {
                nn = format!("{c} ({label})");
            }
            let mut k = 2;
            while present.contains(&nn) {
                nn = format!("{c} ({label} {k})");
                k += 1;
            }
            present.push(nn.clone());
            map.insert(c.clone(), nn.clone());
            new_names.push(nn);
        }
        names.insert(i, map);
        steps.push(Step { name: format!("Expanded {label}"), expr: format!("Table.ExpandTableColumn({}, {}, {}, {})", ident(&merged), quote_text(&nested), text_list(&cols), text_list(&new_names)) });
    }
    Some(build_m(&steps))
}

fn layout(nodes: &[Node], origin: Pos2) -> Vec<Rect> {
    let mut rects = vec![Rect::NOTHING; nodes.len()];
    let mut y = origin.y + 30.0;
    for i in dfs_order(nodes) {
        let mut depth = 0;
        let mut p = nodes.get(i).and_then(|n| n.parent);
        while let Some(q) = p {
            depth += 1;
            p = nodes.get(q).and_then(|n| n.parent);
            if depth > 50 {
                break;
            }
        }
        let h = nodes.get(i).map(Node::height).unwrap_or(BOX_H);
        if let Some(r) = rects.get_mut(i) {
            *r = Rect::from_min_size(pos2(origin.x + 40.0 + depth as f32 * (BOX_W + 90.0), y), vec2(BOX_W, h));
        }
        y += h + 26.0;
    }
    rects
}

/// Draws a Venn icon for a join kind.
fn venn(p: &egui::Painter, c: Pos2, kind: usize, t: &Tokens, hot: bool) {
    let r = 8.0;
    let l = c - vec2(5.0, 0.0);
    let rr = c + vec2(5.0, 0.0);
    p.circle_filled(c, 15.0, if hot { t.accent_soft } else { t.panel });
    p.circle_stroke(c, 15.0, Stroke::new(1.0, if hot { t.accent } else { t.border }));
    let fill = t.accent;
    let (fl, fr) = match kind {
        1 | 4 => (true, false),
        2 | 5 => (false, true),
        3 => (true, true),
        _ => (false, false),
    };
    if fl {
        p.circle_filled(l, r, fill);
    }
    if fr {
        p.circle_filled(rr, r, fill);
    }
    if kind == 0 {
        // The lens where they overlap.
        let pts: Vec<Pos2> = (0..=20)
            .map(|i| {
                let a = (-60.0 + i as f32 * 6.0_f32).to_radians();
                l + vec2(r * a.cos(), r * a.sin())
            })
            .chain((0..=20).map(|i| {
                let a = (120.0 + i as f32 * 6.0_f32).to_radians();
                rr + vec2(r * a.cos(), r * a.sin())
            }))
            .collect();
        p.add(egui::Shape::convex_polygon(pts, fill, Stroke::NONE));
    }
    if kind >= 4 {
        // Anti joins: clear the overlap.
        let pts: Vec<Pos2> = (0..=20)
            .map(|i| {
                let a = (-60.0 + i as f32 * 6.0_f32).to_radians();
                l + vec2(r * a.cos(), r * a.sin())
            })
            .chain((0..=20).map(|i| {
                let a = (120.0 + i as f32 * 6.0_f32).to_radians();
                rr + vec2(r * a.cos(), r * a.sin())
            }))
            .collect();
        p.add(egui::Shape::convex_polygon(pts, if hot { t.accent_soft } else { t.panel }, Stroke::NONE));
    }
    p.circle_stroke(l, r, Stroke::new(1.2, t.accent));
    p.circle_stroke(rr, r, Stroke::new(1.2, t.accent));
}

fn list_item(ui: &mut Ui, t: &Tokens, icon: &str, name: &str, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click_and_drag());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, t.accent_soft);
    }
    super::icons::paint(ui.painter(), Rect::from_min_size(rect.min + vec2(6.0, 5.0), vec2(16.0, 16.0)), icon, t.accent, t.dark);
    ui.painter().with_clip_rect(rect).text(rect.min + vec2(28.0, 13.0), Align2::LEFT_CENTER, name, FontId::proportional(12.5), t.text);
    resp.on_hover_text("Drag onto the canvas (or double-click to add)")
}

/// Adds a table: the root, a join to `parent`, or a union into `union_into`.
fn add_table(app: &mut VizApp, name: &str, parent: Option<usize>, union_into: Option<usize>) {
    if let Some(u) = union_into {
        if let Some(n) = app.pq.canvas.nodes.get_mut(u)
            && !n.tables.iter().any(|x| x == name)
        {
            n.tables.push(name.to_string());
        }
        return;
    }
    if app.pq.canvas.nodes.is_empty() {
        app.pq.canvas.nodes.push(Node { tables: vec![name.to_string()], ..Default::default() });
        if app.pq.canvas.name.is_empty() {
            app.pq.canvas.name = format!("{name} (joined)");
        }
        return;
    }
    let p = parent.unwrap_or(0).min(app.pq.canvas.nodes.len() - 1);
    let pc = app.pq.canvas.nodes.get(p).cloned().map(|n| node_columns(app, &n)).unwrap_or_default();
    let nc = node_columns(app, &Node { tables: vec![name.to_string()], ..Default::default() });
    let clauses = guess_clauses(&pc, &nc);
    let missing = clauses.is_empty();
    app.pq.canvas.nodes.push(Node { tables: vec![name.to_string()], parent: Some(p), join: 0, clauses: if missing { vec![(String::new(), String::new())] } else { clauses } });
    if missing {
        app.pq.canvas.editing = Some(app.pq.canvas.nodes.len() - 1);
    }
}

fn remove_node(c: &mut Canvas, i: usize) {
    if i == 0 {
        c.nodes.clear();
        c.editing = None;
        return;
    }
    // Remove the node and its descendants, re-indexing parents.
    let mut gone = vec![i];
    loop {
        let more: Vec<usize> = c.nodes.iter().enumerate().filter(|(k, n)| !gone.contains(k) && n.parent.is_some_and(|p| gone.contains(&p))).map(|(k, _)| k).collect();
        if more.is_empty() {
            break;
        }
        gone.extend(more);
    }
    let keep: Vec<usize> = (0..c.nodes.len()).filter(|k| !gone.contains(k)).collect();
    let nodes: Vec<Node> = keep
        .iter()
        .filter_map(|k| c.nodes.get(*k).cloned())
        .map(|mut n| {
            n.parent = n.parent.and_then(|p| keep.iter().position(|x| *x == p));
            n
        })
        .collect();
    c.nodes = nodes;
    c.editing = None;
}

/// The sample's star: Sales joined to Product and Store, Store to Targets.
pub fn demo(app: &mut VizApp) {
    if app.pq.canvas.nodes.is_empty() && app.project.sheet("Sales").is_some() {
        add_table(app, "Sales", None, None);
        add_table(app, "Product", Some(0), None);
        add_table(app, "Store", Some(0), None);
        add_table(app, "Targets", Some(2), None);
        if let Some(n) = app.pq.canvas.nodes.get_mut(3) {
            n.join = 1;
        }
        app.pq.canvas.editing = None;
        app.pq.canvas.name = "Sales Joined".into();
    }
}

pub fn show(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    // Top bar.
    egui::Panel::top("ds_top").frame(egui::Frame::new().fill(t.ribbon).inner_margin(egui::Margin::symmetric(10, 8)).stroke(Stroke::new(1.0, t.border))).show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui.button("‹  Power Query Editor").clicked() {
                app.pq.page = super::Page::Editor;
            }
            ui.add_space(8.0);
            let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
            super::icons::paint(ui.painter(), r, "data-source", t.accent, t.dark);
            ui.label(RichText::new("Data Source").strong().size(15.0));
            ui.add_space(16.0);
            ui.label("Query name");
            ui.add(egui::TextEdit::singleline(&mut app.pq.canvas.name).hint_text("Joined data").desired_width(220.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ok = app.pq.canvas.nodes.first().is_some_and(|n| !n.tables.is_empty());
                if ui.add_enabled(ok, egui::Button::new("Save & Open in Power Query").min_size(vec2(0.0, 26.0))).clicked() {
                    save(app, true);
                }
                let r = ui.add_enabled(ok, egui::Button::new(RichText::new("Save as Query").color(Color32::WHITE).strong()).fill(t.accent).min_size(vec2(110.0, 26.0)));
                if r.on_hover_text("Create a query with these joins and unions and load it into the model").clicked() {
                    save(app, false);
                }
                if ui.add_enabled(ok, egui::Button::new("Clear")).clicked() {
                    app.pq.canvas.nodes.clear();
                    app.pq.canvas.editing = None;
                }
            });
        });
    });
    // Left: the tables.
    egui::Panel::left("ds_list").resizable(true).default_size(230.0).min_size(170.0).frame(egui::Frame::new().fill(t.panel).inner_margin(8).stroke(Stroke::new(1.0, t.border))).show(ui, |ui| {
        ui.add(egui::TextEdit::singleline(&mut app.pq.canvas.search).hint_text("Search tables").desired_width(f32::INFINITY));
        ui.add_space(6.0);
        let q = app.pq.canvas.search.to_lowercase();
        let w = ui.available_width();
        let queries: Vec<String> = app.project.queries.queries.iter().map(|q| q.name.clone()).filter(|n| q.is_empty() || n.to_lowercase().contains(&q)).collect();
        let sheets: Vec<String> = app.project.sheets.iter().map(|s| s.name.clone()).filter(|n| q.is_empty() || n.to_lowercase().contains(&q)).collect();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            forms::caption(ui, t, "Queries");
            for n in &queries {
                let r = list_item(ui, t, "query", n, w);
                if r.drag_started() {
                    app.pq.canvas.drag = Some(n.clone());
                }
                if r.double_clicked() {
                    let last = app.pq.canvas.nodes.len().checked_sub(1);
                    add_table(app, n, last.map(|_| 0), None);
                }
            }
            ui.add_space(8.0);
            forms::caption(ui, t, "Data sheets");
            for n in &sheets {
                let r = list_item(ui, t, "sheet", n, w);
                if r.drag_started() {
                    app.pq.canvas.drag = Some(n.clone());
                }
                if r.double_clicked() {
                    add_table(app, n, Some(0), None);
                }
            }
            ui.add_space(8.0);
            forms::caption(ui, t, "Connections");
            if ui.add(egui::Button::new("+  Add a connection…").frame(false)).on_hover_text("Get data: the new query appears in the list").clicked() {
                super::get_data(app, "more");
            }
            for r in app.pq.recent.clone().iter().take(6) {
                let label = std::path::Path::new(&r.target).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| r.target.clone());
                if list_item(ui, t, "web", &label, w).on_hover_text(&r.target).clicked() {
                    super::getdata::open_recent(app, r);
                }
            }
        });
    });
    // Bottom: the preview.
    let m = build(app, &app.pq.canvas.nodes.clone());
    egui::Panel::bottom("ds_preview").resizable(true).default_size(260.0).min_size(120.0).frame(egui::Frame::new().fill(t.panel).inner_margin(8).stroke(Stroke::new(1.0, t.border))).show(ui, |ui| {
        match &m {
            None => {
                ui.label(RichText::new("Drag a table onto the canvas to start.").color(t.muted));
            }
            Some(m) => {
                let r = match &app.pq.canvas.preview {
                    Some((k, r)) if k == m => r.clone(),
                    _ => {
                        let r = super::eval_m(app, m).and_then(|v| super::as_table(v).map_err(|s| s.unwrap_or_default()));
                        app.pq.canvas.preview = Some((m.clone(), r.clone()));
                        r
                    }
                };
                match r {
                    Ok(tb) => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{} fields · {} rows", tb.columns.len(), tb.rows.len())).strong());
                            ui.label(RichText::new("(first 100 rows shown)").size(11.0).color(t.muted));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Copy M").clicked() {
                                    ui.ctx().copy_text(m.clone());
                                }
                            });
                        });
                        super::getdata::preview_grid(ui, t, &tb, 100, "ds_grid");
                    }
                    Err(e) => {
                        ui.colored_label(t.danger, e);
                    }
                }
            }
        }
    });
    // The canvas.
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.canvas)).show(ui, |ui| {
        canvas_area(app, ui, t);
    });
    join_editor(app, ui.ctx(), t);
}

fn canvas_area(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let area = ui.available_rect_before_wrap();
    let resp = ui.allocate_rect(area, Sense::hover());
    let painter = ui.painter_at(area);
    // Dotted grid.
    let mut x = area.min.x + 10.0;
    while x < area.max.x {
        let mut y = area.min.y + 10.0;
        while y < area.max.y {
            painter.circle_filled(pos2(x, y), 0.8, t.border);
            y += 20.0;
        }
        x += 20.0;
    }
    let nodes = app.pq.canvas.nodes.clone();
    let rects = layout(&nodes, area.min);
    let pointer = ui.ctx().pointer_latest_pos();
    let dragging = app.pq.canvas.drag.clone();
    if nodes.is_empty() {
        let c = area.center();
        let r = Rect::from_center_size(c, vec2(360.0, 120.0));
        let hot = dragging.is_some() && pointer.is_some_and(|p| area.contains(p));
        painter.rect(r, 10.0, if hot { t.accent_soft } else { t.panel }, Stroke::new(1.5, if hot { t.accent } else { t.border }), StrokeKind::Inside);
        painter.text(c - vec2(0.0, 12.0), Align2::CENTER_CENTER, "Drag tables here", FontId::proportional(16.0), t.text);
        painter.text(c + vec2(0.0, 14.0), Align2::CENTER_CENTER, "Drop beside a table to join, on its lower half to union", FontId::proportional(11.5), t.muted);
    }
    // Links with Venn icons.
    let mut venn_click: Option<usize> = None;
    for (i, n) in nodes.iter().enumerate() {
        let (Some(p), Some(r)) = (n.parent, rects.get(i)) else { continue };
        let Some(pr) = rects.get(p) else { continue };
        let a = pos2(pr.max.x, pr.min.y + BOX_H / 2.0);
        let b = pos2(r.min.x, r.min.y + BOX_H / 2.0);
        let mid_x = a.x + 34.0;
        painter.add(egui::Shape::line(vec![a, pos2(mid_x, a.y), pos2(mid_x, b.y), b], Stroke::new(1.5, t.muted)));
        let vc = pos2(mid_x + (b.x - mid_x) / 2.0, b.y);
        let vr = Rect::from_center_size(vc, vec2(30.0, 30.0));
        let vresp = ui.interact(vr, ui.scope_id().with(("venn", i)), Sense::click());
        let bad = n.clauses.iter().all(|(a, b)| a.is_empty() || b.is_empty());
        venn(&painter, vc, n.join, t, vresp.hovered() || app.pq.canvas.editing == Some(i));
        if bad {
            painter.circle_filled(vc + vec2(11.0, -11.0), 5.0, t.danger);
            painter.text(vc + vec2(11.0, -11.0), Align2::CENTER_CENTER, "!", FontId::proportional(9.0), Color32::WHITE);
        }
        let tip = format!("{} join on {}", JOINS.get(n.join).map(|j| j.0).unwrap_or("Inner"), n.clauses.iter().map(|(a, b)| format!("{a} = {b}")).collect::<Vec<_>>().join(", "));
        if vresp.on_hover_text(tip).clicked() {
            venn_click = Some(i);
        }
    }
    if let Some(i) = venn_click {
        app.pq.canvas.editing = Some(i);
    }
    // Nodes.
    let mut drop_target: Option<(usize, bool)> = None;
    let mut remove: Option<usize> = None;
    let mut remove_member: Option<(usize, usize)> = None;
    for (i, n) in nodes.iter().enumerate() {
        let Some(r) = rects.get(i).copied() else { continue };
        let hovered_drop = dragging.is_some() && pointer.is_some_and(|p| r.expand(6.0).contains(p));
        let union_zone = hovered_drop && pointer.is_some_and(|p| p.y > r.min.y + r.height() * 0.6);
        if hovered_drop {
            drop_target = Some((i, union_zone));
        }
        let fill = if hovered_drop { t.accent_soft } else { t.panel };
        painter.rect(r, 6.0, fill, Stroke::new(if hovered_drop { 2.0 } else { 1.0 }, if hovered_drop { t.accent } else { t.border }), StrokeKind::Inside);
        painter.rect_filled(Rect::from_min_size(r.min, vec2(4.0, r.height())), 2.0, if i == 0 { t.accent } else { t.good });
        let icon = if n.tables.len() > 1 { "union" } else if app.project.sheet(n.tables.first().map(String::as_str).unwrap_or("")).is_some() { "sheet" } else { "query" };
        super::icons::paint(&painter, Rect::from_min_size(r.min + vec2(12.0, 10.0), vec2(18.0, 18.0)), icon, t.accent, t.dark);
        painter.with_clip_rect(r.shrink(2.0)).text(r.min + vec2(38.0, BOX_H / 2.0), Align2::LEFT_CENTER, n.label(), FontId::proportional(13.0), t.text);
        if n.tables.len() > 1 {
            for (k, m) in n.tables.iter().enumerate() {
                let mr = Rect::from_min_size(pos2(r.min.x + 14.0, r.min.y + BOX_H + k as f32 * MEMBER_H), vec2(r.width() - 20.0, MEMBER_H));
                painter.text(mr.left_center() + vec2(14.0, 0.0), Align2::LEFT_CENTER, m, FontId::proportional(11.5), t.muted);
                painter.circle_filled(mr.left_center() + vec2(4.0, 0.0), 2.0, t.muted);
                let mresp = ui.interact(mr, ui.scope_id().with(("member", i, k)), Sense::click());
                mresp.context_menu(|ui| {
                    if ui.button("Remove from union").clicked() {
                        remove_member = Some((i, k));
                        ui.close();
                    }
                });
            }
        }
        if union_zone {
            let z = Rect::from_min_max(pos2(r.min.x, r.max.y - 12.0), r.max);
            painter.rect_filled(z, 4.0, t.accent.gamma_multiply(0.35));
            painter.text(pos2(r.center().x, r.max.y + 9.0), Align2::CENTER_CENTER, "Union", FontId::proportional(11.0), t.accent);
        } else if hovered_drop {
            painter.text(pos2(r.max.x + 30.0, r.center().y), Align2::LEFT_CENTER, "Join", FontId::proportional(11.0), t.accent);
        }
        let nresp = ui.interact(r, ui.scope_id().with(("node", i)), Sense::click());
        let cols = node_columns(app, n).len();
        let nresp = nresp.on_hover_text(format!("{}{} columns", if i == 0 { "Root table · " } else { "" }, cols));
        if nresp.double_clicked() && i > 0 {
            app.pq.canvas.editing = Some(i);
        }
        nresp.context_menu(|ui| {
            if i > 0 && ui.button("Edit Join…").clicked() {
                app.pq.canvas.editing = Some(i);
                ui.close();
            }
            if n.tables.len() == 1 && ui.button("Open in Power Query").clicked() {
                let q = n.tables.first().cloned().unwrap_or_default();
                if app.project.queries.query(&q).is_some() {
                    super::open_editor(app, Some(&q));
                }
                ui.close();
            }
            if ui.button("Remove").clicked() {
                remove = Some(i);
                ui.close();
            }
        });
    }
    if let Some(i) = remove {
        remove_node(&mut app.pq.canvas, i);
    }
    if let Some((i, k)) = remove_member
        && let Some(n) = app.pq.canvas.nodes.get_mut(i)
        && n.tables.len() > 1
    {
        n.tables.remove(k);
    }
    // Dragging: a ghost at the pointer; dropping adds the table.
    if let Some(name) = dragging {
        if let Some(p) = pointer {
            let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("ds_ghost"));
            let gp = ui.ctx().layer_painter(layer);
            let r = Rect::from_min_size(p + vec2(10.0, 6.0), vec2(170.0, 28.0));
            gp.rect(r, 5.0, t.panel, Stroke::new(1.0, t.accent), StrokeKind::Inside);
            gp.text(r.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, &name, FontId::proportional(12.5), t.text);
        }
        if ui.input(|i| i.pointer.any_released()) {
            app.pq.canvas.drag = None;
            if pointer.is_some_and(|p| area.contains(p)) {
                match drop_target {
                    Some((i, true)) => add_table(app, &name, None, Some(i)),
                    Some((i, false)) => add_table(app, &name, Some(i), None),
                    None => {
                        // Beside the nearest table (by height on the canvas), else under the root.
                        let near = pointer.and_then(|p| rects.iter().enumerate().filter(|(_, r)| p.x > r.max.x).min_by(|a, b| (a.1.center().y - p.y).abs().total_cmp(&(b.1.center().y - p.y).abs())).map(|(i, _)| i));
                        add_table(app, &name, near.or(Some(0)), None);
                    }
                }
            }
        }
        ui.ctx().request_repaint();
    }
    let _ = resp;
}

/// The join editor: kind (Venn buttons) and clauses.
fn join_editor(app: &mut VizApp, ctx: &egui::Context, t: &Tokens) {
    let Some(i) = app.pq.canvas.editing else { return };
    let Some(n) = app.pq.canvas.nodes.get(i).cloned() else {
        app.pq.canvas.editing = None;
        return;
    };
    let Some(p) = n.parent else {
        app.pq.canvas.editing = None;
        return;
    };
    let pn = app.pq.canvas.nodes.get(p).cloned().unwrap_or_default();
    let left_cols = node_columns(app, &pn);
    let right_cols = node_columns(app, &n);
    let mut node = n.clone();
    let mut open = true;
    let mut done = false;
    egui::Window::new(RichText::new("Join").strong()).open(&mut open).collapsible(false).resizable(false).default_width(420.0).anchor(Align2::RIGHT_TOP, vec2(-20.0, 120.0)).show(ctx, |ui| {
        ui.horizontal(|ui| {
            for (k, (label, _)) in JOINS.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(64.0, 54.0), Sense::click());
                let sel = node.join == k;
                if sel {
                    ui.painter().rect(rect, 4.0, t.accent_soft, Stroke::new(1.0, t.accent), StrokeKind::Inside);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 4.0, t.panel_alt);
                }
                venn(ui.painter(), rect.center() - vec2(0.0, 8.0), k, t, sel);
                ui.painter().text(pos2(rect.center().x, rect.max.y - 8.0), Align2::CENTER_CENTER, *label, FontId::proportional(10.5), t.text);
                if resp.clicked() {
                    node.join = k;
                }
            }
        });
        ui.add_space(6.0);
        egui::Grid::new("ds_clauses").num_columns(4).spacing(vec2(6.0, 4.0)).show(ui, |ui| {
            ui.label(RichText::new(pn.label()).strong());
            ui.label("");
            ui.label(RichText::new(node.label()).strong());
            ui.label("");
            ui.end_row();
            let mut remove = None;
            for (k, (a, b)) in node.clauses.iter_mut().enumerate() {
                forms::combo(ui, ("ds_l", k), a, &left_cols, 150.0);
                ui.label("=");
                forms::combo(ui, ("ds_r", k), b, &right_cols, 150.0);
                if forms::glyph_button(ui, t, 'x', t.muted, "Remove this clause").clicked() {
                    remove = Some(k);
                }
                ui.end_row();
            }
            if let Some(k) = remove {
                node.clauses.remove(k);
            }
        });
        if ui.small_button("Add join clause").clicked() {
            node.clauses.push((String::new(), String::new()));
        }
        if node.clauses.iter().all(|(a, b)| a.is_empty() || b.is_empty()) {
            forms::note(ui, t, "Pick the matching columns of both tables.", true);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if primary(ui, t, "Done").clicked() {
                    done = true;
                }
            });
        });
    });
    if let Some(slot) = app.pq.canvas.nodes.get_mut(i) {
        *slot = node;
    }
    if !open || done {
        app.pq.canvas.editing = None;
    }
}

fn save(app: &mut VizApp, open: bool) {
    let nodes = app.pq.canvas.nodes.clone();
    let Some(m) = build(app, &nodes) else { return };
    if let Ok(Value::Error(e)) = super::eval_m(app, &m).map(|v| match v {
        Value::Error(e) => Value::Error(e),
        _ => Value::Null,
    }) {
        app.error(e.to_string());
        return;
    }
    let name = if app.pq.canvas.name.trim().is_empty() { "Joined".to_string() } else { app.pq.canvas.name.trim().to_string() };
    let q = super::new_query(app, &name, m, true, Some("Data Sources".into()));
    if open {
        super::open_editor(app, Some(&q));
    } else {
        super::apply_to_model(app);
        app.toast(format!("Created query {q} (joins and unions are Power Query steps)"));
    }
}
