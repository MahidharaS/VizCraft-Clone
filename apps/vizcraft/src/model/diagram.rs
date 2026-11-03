//! Model view: the semantic model as a diagram. Table cards (coloured header for fact,
//! dimension, calculated, parameter and date tables; columns with type icons; measures;
//! hierarchies) are dragged around and remembered per layout tab; relationships are bezier lines
//! between the key columns with cardinality markers (1 / *), filter-direction arrows and dashes
//! when inactive. Drag a column onto another table's column to relate them; click a line to see
//! its properties, double-click to edit it. Scroll to pan, Ctrl+scroll to zoom, drag the
//! background to pan.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use vizcraft_core::doc::*;

use super::{Act, ColMeta, Dir, PaneMode, Sel, TableKind, TableMeta, data_pane, fmt_count, glyph_icon, icon_btn, paint_type_icon, run_act, strip_group, tables_meta, tri};
use crate::app::VizApp;
use crate::theme::Tokens;
use crate::widgets::big_button;

const CARD_W: f32 = 236.0;
const HEAD_H: f32 = 44.0;
const ROW_H: f32 = 22.0;
const SECTION_H: f32 = 20.0;
/// Items a collapsed card shows before "n more".
const MAX_ITEMS: usize = 12;

pub struct DiagramState {
    pub layout: usize,
    pan: Vec2,
    fit: bool,
    drag_card: Option<String>,
    rel_drag: Option<(String, String)>,
    renaming: Option<(usize, String)>,
    /// Cards showing every item.
    expanded: BTreeSet<String>,
    /// Cards showing only their header.
    collapsed: BTreeSet<String>,
    canvas: Rect,
}

impl Default for DiagramState {
    fn default() -> Self {
        DiagramState { layout: 0, pan: Vec2::ZERO, fit: true, drag_card: None, rel_drag: None, renaming: None, expanded: BTreeSet::new(), collapsed: BTreeSet::new(), canvas: Rect::NOTHING }
    }
}

/// One line of a card.
#[derive(Clone)]
enum Item {
    Section(&'static str),
    Column(ColMeta),
    Measure(String, bool),
    Hierarchy(String, usize),
    More(usize),
}

fn items(m: &TableMeta, expanded: bool, collapsed: bool) -> Vec<Item> {
    if collapsed {
        return vec![];
    }
    let mut v = Vec::new();
    let cols: Vec<Item> = m.columns.iter().cloned().map(Item::Column).collect();
    let meas: Vec<Item> = m.measures.iter().map(|(n, h)| Item::Measure(n.clone(), *h)).collect();
    let hier: Vec<Item> = m.hierarchies.iter().map(|h| Item::Hierarchy(h.name.clone(), h.levels.len())).collect();
    let total = cols.len() + meas.len() + hier.len();
    let mut budget = if expanded || total <= MAX_ITEMS + 2 { usize::MAX } else { MAX_ITEMS };
    let mut shown = 0;
    for (label, list) in [("COLUMNS", cols), ("MEASURES", meas), ("HIERARCHIES", hier)] {
        if list.is_empty() || budget == 0 {
            continue;
        }
        if label != "COLUMNS" {
            v.push(Item::Section(label));
        }
        for it in list {
            if budget == 0 {
                break;
            }
            v.push(it);
            shown += 1;
            budget = budget.saturating_sub(1);
        }
    }
    if shown < total {
        v.push(Item::More(total - shown));
    }
    v
}

fn item_h(it: &Item) -> f32 {
    match it {
        Item::Section(_) => SECTION_H,
        _ => ROW_H,
    }
}

fn card_h(its: &[Item]) -> f32 {
    HEAD_H + its.iter().map(item_h).sum::<f32>() + if its.is_empty() { 0.0 } else { 8.0 }
}

/// Makes sure there is an "All tables" layout with a position for every table.
fn ensure_layout(app: &mut VizApp, metas: &[TableMeta]) {
    if app.project.model.layouts.is_empty() {
        app.project.model.layouts.push(DiagramLayout { name: "All tables".into(), zoom: 1.0, ..DiagramLayout::default() });
    }
    let st = &mut app.model_ui.diagram;
    if st.layout >= app.project.model.layouts.len() {
        st.layout = 0;
    }
    let Some(first) = app.project.model.layouts.first_mut() else { return };
    let missing: Vec<&TableMeta> = metas.iter().filter(|m| !first.positions.contains_key(&m.name)).collect();
    if missing.is_empty() {
        return;
    }
    if first.positions.is_empty() {
        let pos = arrange(metas, &app.project.model.relationships);
        if let Some(first) = app.project.model.layouts.first_mut() {
            first.positions = pos;
        }
        return;
    }
    let bottom = metas
        .iter()
        .filter_map(|m| first.positions.get(&m.name).map(|p| p.1 + card_h(&items(m, false, false))))
        .fold(0.0f32, f32::max)
        + 60.0;
    for (i, m) in missing.iter().enumerate() {
        first.positions.insert(m.name.clone(), (40.0 + i as f32 * (CARD_W + 60.0), bottom));
    }
}

/// A star layout: fact tables in the middle column, the dimensions they use alternately left and
/// right, everything else in rows underneath.
pub fn arrange(metas: &[TableMeta], rels: &[Relationship]) -> BTreeMap<String, (f32, f32)> {
    let mut out = BTreeMap::new();
    let h = |m: &TableMeta| card_h(&items(m, false, false));
    let gap_x = 140.0;
    let (lx, mx, rx) = (40.0, 40.0 + CARD_W + gap_x, 40.0 + 2.0 * (CARD_W + gap_x));
    let (mut ly, mut my, mut ry) = (40.0f32, 40.0f32, 40.0f32);
    let mut side = false;
    let mut placed: BTreeSet<String> = BTreeSet::new();
    for f in metas.iter().filter(|m| m.kind == TableKind::Fact) {
        out.insert(f.name.clone(), (mx, my));
        my += h(f) + 60.0;
        placed.insert(f.name.clone());
        // Its dimensions.
        for r in rels.iter().filter(|r| r.from_table == f.name || r.to_table == f.name) {
            let other = if r.from_table == f.name { &r.to_table } else { &r.from_table };
            if placed.contains(other) {
                continue;
            }
            let Some(om) = metas.iter().find(|m| &m.name == other) else { continue };
            if side {
                out.insert(other.clone(), (rx, ry));
                ry += h(om) + 50.0;
            } else {
                out.insert(other.clone(), (lx, ly));
                ly += h(om) + 50.0;
            }
            side = !side;
            placed.insert(other.clone());
        }
    }
    let below = ly.max(my).max(ry) + 20.0;
    let mut k = 0;
    let mut row_h = 0.0f32;
    let mut y = below;
    for m in metas.iter().filter(|m| !placed.contains(&m.name)) {
        if k > 0 && k % 4 == 0 {
            y += row_h + 50.0;
            row_h = 0.0;
        }
        out.insert(m.name.clone(), (40.0 + (k % 4) as f32 * (CARD_W + 60.0), y));
        row_h = row_h.max(h(m));
        k += 1;
    }
    out
}

/// A cubic bezier between two anchors leaving/entering cards horizontally.
fn bezier(a: Pos2, a_dir: f32, b: Pos2, b_dir: f32) -> Vec<Pos2> {
    let d = ((b.x - a.x).abs() * 0.5).max(50.0);
    let (c1, c2) = (a + vec2(a_dir * d, 0.0), b + vec2(b_dir * d, 0.0));
    (0..=32)
        .map(|i| {
            let t = i as f32 / 32.0;
            let u = 1.0 - t;
            let p = a.to_vec2() * u * u * u + c1.to_vec2() * 3.0 * u * u * t + c2.to_vec2() * 3.0 * u * t * t + b.to_vec2() * t * t * t;
            p.to_pos2()
        })
        .collect()
}

fn dist_to_poly(p: Pos2, pts: &[Pos2]) -> f32 {
    pts.windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let ab = b - a;
            let t = if ab.length_sq() < 1e-6 { 0.0 } else { ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) };
            (p - (a + ab * t)).length()
        })
        .fold(f32::INFINITY, f32::min)
}

/// The point at a fraction of a polyline's length, and the direction there.
fn along(pts: &[Pos2], frac: f32) -> (Pos2, Vec2) {
    let lens: Vec<f32> = pts.windows(2).map(|w| (w[1] - w[0]).length()).collect();
    let total: f32 = lens.iter().sum();
    let mut target = total * frac;
    for (i, l) in lens.iter().enumerate() {
        if target <= *l || i + 1 == lens.len() {
            let (a, b) = (pts[i], pts[i + 1]);
            let t = if *l > 0.0 { (target / l).clamp(0.0, 1.0) } else { 0.0 };
            return (a + (b - a) * t, (b - a).normalized());
        }
        target -= l;
    }
    (pts.first().copied().unwrap_or(Pos2::ZERO), Vec2::X)
}

fn arrow(p: &egui::Painter, at: Pos2, dir: Vec2, color: Color32, z: f32) {
    let d = dir.normalized() * 6.0 * z;
    let n = vec2(-d.y, d.x) * 0.75;
    p.add(Shape::convex_polygon(vec![at + d, at - d + n, at - d - n], color, Stroke::NONE));
}

fn dashed(p: &egui::Painter, pts: &[Pos2], stroke: Stroke, dash: f32) {
    let mut on = true;
    let mut left = dash;
    for w in pts.windows(2) {
        let (mut a, b) = (w[0], w[1]);
        let mut seg = (b - a).length();
        let dir = (b - a).normalized();
        while seg > 0.0 {
            let step = seg.min(left);
            let nb = a + dir * step;
            if on {
                p.line_segment([a, nb], stroke);
            }
            a = nb;
            seg -= step;
            left -= step;
            if left <= 0.0 {
                on = !on;
                left = if on { dash } else { dash * 0.7 };
            }
        }
    }
}

struct CardGeom {
    rect: Rect,
    /// Item rects (screen) by column name.
    rows: HashMap<String, Rect>,
}

pub fn show(app: &mut VizApp, ui: &mut Ui) {
    let t = app.tokens();
    let metas = tables_meta(&app.project, &app.built);
    ensure_layout(app, &metas);
    super::table_view::load_buffers_pub(app);
    egui::Panel::top("mv_tools").frame(egui::Frame::new().fill(t.ribbon).inner_margin(egui::Margin { left: 10, right: 10, top: 6, bottom: 2 }).stroke(Stroke::new(1.0, t.border))).show(ui, |ui| {
        ui.horizontal(|ui| toolbar(app, ui, &t, &metas));
    });
    let mut acts = Vec::new();
    egui::Panel::right("mv_data").default_size(250.0).size_range(200.0..=420.0).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::same(10)).stroke(Stroke::new(1.0, t.border))).show(ui, |ui| {
        acts = data_pane(app, ui, PaneMode::Model);
    });
    egui::Panel::right("mv_props").default_size(290.0).size_range(240.0..=420.0).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::same(12)).stroke(Stroke::new(1.0, t.border))).show(ui, |ui| {
        properties(app, ui, &t);
    });
    egui::Panel::bottom("mv_tabs").exact_size(34.0).frame(egui::Frame::new().fill(t.panel_alt).inner_margin(egui::Margin::symmetric(8, 4)).stroke(Stroke::new(1.0, t.border))).show(ui, |ui| layout_tabs(app, ui, &t));
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.canvas)).show(ui, |ui| canvas(app, ui, &t, &metas));
    for a in acts {
        run_act(app, a);
    }
}

fn toolbar(app: &mut VizApp, ui: &mut Ui, t: &Tokens, metas: &[TableMeta]) {
    strip_group(ui, t, "Relationships", |ui| {
        if big_button(ui, t, "relationship", "Manage relationships", "Create, edit, autodetect and delete relationships").clicked() {
            super::manage_relationships(app);
        }
    });
    strip_group(ui, t, "Calculations", |ui| {
        if big_button(ui, t, "measure", "New measure", "").clicked() {
            super::new_measure(app, None);
        }
        if big_button(ui, t, "column", "New column", "").clicked() {
            super::new_column(app, None);
        }
        if big_button(ui, t, "table", "New table", "").clicked() {
            super::new_table(app);
        }
        if big_button(ui, t, "sparkle", "Quick measure", "").clicked() {
            super::quick_measure(app);
        }
    });
    strip_group(ui, t, "Security", |ui| {
        if big_button(ui, t, "security", "Manage roles", "Row-level security roles").clicked() {
            super::manage_roles(app);
        }
        if big_button(ui, t, "security", "View as", "See the report as a role").clicked() {
            super::view_as(app);
        }
    });
    strip_group(ui, t, "Layout", |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                if ui.button("Auto-layout").on_hover_text("Arrange the tables as a star: facts in the middle, dimensions around").clicked() {
                    app.checkpoint("Auto-layout");
                    let pos = arrange(metas, &app.project.model.relationships);
                    let li = app.model_ui.diagram.layout;
                    if let Some(l) = app.project.model.layouts.get_mut(li) {
                        for (k, v) in pos {
                            if l.tables.is_empty() || l.tables.contains(&k) {
                                l.positions.insert(k, v);
                            }
                        }
                    }
                    app.model_ui.diagram.fit = true;
                }
                if ui.button("Fit to screen").clicked() {
                    app.model_ui.diagram.fit = true;
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Collapse all").on_hover_text("Show only the table headers").clicked() {
                    app.model_ui.diagram.collapsed = metas.iter().map(|m| m.name.clone()).collect();
                    app.model_ui.diagram.expanded.clear();
                }
                if ui.button("Expand all").clicked() {
                    app.model_ui.diagram.collapsed.clear();
                    app.model_ui.diagram.expanded = metas.iter().map(|m| m.name.clone()).collect();
                }
            });
            let li = app.model_ui.diagram.layout;
            if let Some(l) = app.project.model.layouts.get_mut(li) {
                if l.zoom <= 0.0 {
                    l.zoom = 1.0;
                }
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Zoom").size(11.0).color(t.muted));
                    ui.add(egui::Slider::new(&mut l.zoom, 0.3..=2.0).show_value(false));
                    ui.label(egui::RichText::new(format!("{:.0}%", l.zoom * 100.0)).size(11.0));
                });
            }
        });
    });
}

fn layout_tabs(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    ui.horizontal_centered(|ui| {
        let n = app.project.model.layouts.len();
        let mut remove = None;
        for i in 0..n {
            let name = app.project.model.layouts.get(i).map(|l| l.name.clone()).unwrap_or_default();
            let active = app.model_ui.diagram.layout == i;
            if let Some((ri, buf)) = &mut app.model_ui.diagram.renaming
                && *ri == i
            {
                let r = ui.add(egui::TextEdit::singleline(buf).desired_width(120.0));
                r.request_focus();
                if r.lost_focus() {
                    let new = buf.trim().to_string();
                    if !new.is_empty() {
                        app.checkpoint("Rename layout");
                        if let Some(l) = app.project.model.layouts.get_mut(i) {
                            l.name = new;
                        }
                    }
                    app.model_ui.diagram.renaming = None;
                }
                continue;
            }
            let text = egui::RichText::new(&name).size(12.0).color(if active { t.accent } else { t.text });
            let r = ui.add(egui::Button::new(text).fill(if active { t.panel } else { Color32::TRANSPARENT }).stroke(if active { Stroke::new(1.0, t.border) } else { Stroke::NONE }));
            if active {
                let rr = r.rect;
                ui.painter().line_segment([rr.left_bottom() + vec2(4.0, 0.0), rr.right_bottom() - vec2(4.0, 0.0)], Stroke::new(2.0, t.accent));
            }
            if r.clicked() {
                app.model_ui.diagram.layout = i;
                app.model_ui.diagram.fit = true;
            }
            if r.double_clicked() && i > 0 {
                app.model_ui.diagram.renaming = Some((i, name.clone()));
            }
            r.context_menu(|ui| {
                if i > 0 && ui.button("Rename").clicked() {
                    app.model_ui.diagram.renaming = Some((i, name.clone()));
                    ui.close();
                }
                if i > 0 && ui.button("Delete layout").clicked() {
                    remove = Some(i);
                    ui.close();
                }
                if i == 0 {
                    ui.label(egui::RichText::new("The main layout always shows every table.").color(t.muted));
                }
            });
        }
        if icon_btn(ui, t, "plus", "New layout", Some(t.accent)).clicked() {
            app.checkpoint("New layout");
            let name = (2..).map(|k| format!("Layout {k}")).find(|n| !app.project.model.layouts.iter().any(|l| &l.name == n)).unwrap_or_default();
            app.project.model.layouts.push(DiagramLayout { name, zoom: 1.0, ..DiagramLayout::default() });
            app.model_ui.diagram.layout = app.project.model.layouts.len() - 1;
        }
        if let Some(i) = remove {
            app.checkpoint("Delete layout");
            app.project.model.layouts.remove(i);
            app.model_ui.diagram.layout = 0;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let legend = [(TableKind::Fact, "Fact"), (TableKind::Dimension, "Dimension"), (TableKind::Date, "Date"), (TableKind::Calculated, "Calculated"), (TableKind::Parameter, "Parameter")];
            for (k, l) in legend.iter().rev() {
                ui.label(egui::RichText::new(*l).size(11.0).color(t.muted));
                let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                ui.painter().rect_filled(r, 2.0, k.color(t));
                ui.add_space(6.0);
            }
        });
    });
}

fn canvas(app: &mut VizApp, ui: &mut Ui, t: &Tokens, metas: &[TableMeta]) {
    let li = app.model_ui.diagram.layout;
    let Some(layout) = app.project.model.layouts.get(li).cloned() else { return };
    let shown: Vec<&TableMeta> = metas.iter().filter(|m| li == 0 || layout.tables.iter().any(|x| x == &m.name)).collect();
    let rect = ui.available_rect_before_wrap();
    let bg = ui.interact(rect, ui.scope_id().with("mv_bg"), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let st = &mut app.model_ui.diagram;
    st.canvas = rect;
    let mut z = if layout.zoom > 0.0 { layout.zoom } else { 1.0 };
    // Item lists and world rects.
    let its: Vec<(&TableMeta, Vec<Item>)> =
        shown.iter().map(|m| (*m, items(m, st.expanded.contains(&m.name), st.collapsed.contains(&m.name)))).collect();
    let world = |m: &TableMeta, its: &[Item]| -> Option<Rect> { layout.positions.get(&m.name).map(|(x, y)| Rect::from_min_size(pos2(*x, *y), vec2(CARD_W, card_h(its)))) };
    if st.fit {
        st.fit = false;
        let bounds = its.iter().filter_map(|(m, i)| world(m, i)).fold(Rect::NOTHING, |a, b| a.union(b));
        if bounds.is_positive() {
            let zz = ((rect.width() - 80.0) / bounds.width()).min((rect.height() - 80.0) / bounds.height()).clamp(0.3, 1.25);
            z = zz;
            st.pan = rect.center().to_vec2() - rect.min.to_vec2() - bounds.center().to_vec2() * zz;
            if let Some(l) = app.project.model.layouts.get_mut(li) {
                l.zoom = zz;
            }
        }
    }
    let st = &mut app.model_ui.diagram;
    let to_screen = |p: Pos2| rect.min + st.pan + p.to_vec2() * z;
    // Background: dot grid.
    painter.rect_filled(rect, 0.0, t.canvas);
    let step = 22.0 * z;
    if step > 6.0 {
        let off = rect.min + st.pan;
        let dot = if t.dark { Color32::from_rgb(0x2A, 0x30, 0x3C) } else { Color32::from_rgb(0xCB, 0xD2, 0xDC) };
        let mut x = off.x + ((rect.left() - off.x) / step).floor() * step;
        while x < rect.right() {
            let mut y = off.y + ((rect.top() - off.y) / step).floor() * step;
            while y < rect.bottom() {
                painter.circle_filled(pos2(x, y), 1.0, dot);
                y += step;
            }
            x += step;
        }
    }
    // Card geometry on screen.
    let mut geoms: HashMap<String, CardGeom> = HashMap::new();
    for (m, list) in &its {
        let Some(w) = world(m, list) else { continue };
        let r = Rect::from_min_max(to_screen(w.min), to_screen(w.max));
        let mut rows = HashMap::new();
        let mut y = r.top() + HEAD_H * z;
        for it in list {
            let h = item_h(it) * z;
            if let Item::Column(c) = it {
                rows.insert(c.name.clone(), Rect::from_min_size(pos2(r.left(), y), vec2(r.width(), h)));
            }
            y += h;
        }
        geoms.insert(m.name.clone(), CardGeom { rect: r, rows });
    }
    // Relationship lines.
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let sel = app.model_ui.sel.clone();
    let mut hit_rel: Option<(u64, f32)> = None;
    for r in &app.project.model.relationships {
        let (Some(ga), Some(gb)) = (geoms.get(&r.from_table), geoms.get(&r.to_table)) else { continue };
        let anchor = |g: &CardGeom, col: &str| g.rows.get(col).map(|rr| rr.center().y).unwrap_or(g.rect.top() + HEAD_H * z * 0.5);
        let (ya, yb) = (anchor(ga, &r.from_column), anchor(gb, &r.to_column));
        let a_left = ga.rect.center().x > gb.rect.center().x;
        let (xa, da) = if a_left { (ga.rect.left(), -1.0) } else { (ga.rect.right(), 1.0) };
        let (xb, db) = if a_left { (gb.rect.right(), 1.0) } else { (gb.rect.left(), -1.0) };
        // Cards stacked vertically: leave and enter on the same side.
        let overlap = (ga.rect.center().x - gb.rect.center().x).abs() < CARD_W * z * 0.8;
        let (xb, db) = if overlap { (gb.rect.right(), 1.0) } else { (xb, db) };
        let (xa, da) = if overlap { (ga.rect.right(), 1.0) } else { (xa, da) };
        let pts = bezier(pos2(xa, ya), da, pos2(xb, yb), db);
        let d = pointer.map(|p| dist_to_poly(p, &pts)).unwrap_or(f32::INFINITY);
        let hovered = d < 6.0 && rect.contains(pointer.unwrap_or_default());
        if hovered && hit_rel.is_none_or(|(_, hd)| d < hd) {
            hit_rel = Some((r.id, d));
        }
        let selected = sel == Sel::Relationship(r.id);
        let color = if selected {
            t.accent
        } else if hovered {
            t.accent.gamma_multiply(0.75)
        } else if t.dark {
            Color32::from_rgb(0x7A, 0x86, 0x9A)
        } else {
            Color32::from_rgb(0x8A, 0x95, 0xA6)
        };
        let width = if selected { 2.6 } else if hovered { 2.2 } else { 1.6 };
        if r.active {
            painter.add(Shape::line(pts.clone(), Stroke::new(width, color)));
        } else {
            dashed(&painter, &pts, Stroke::new(width, color), 7.0 * z.max(0.6));
        }
        // Cardinality markers.
        let (many_a, many_b) = match r.cardinality {
            Cardinality::ManyToOne => (true, false),
            Cardinality::OneToMany => (false, true),
            Cardinality::OneToOne => (false, false),
            Cardinality::ManyToMany => (true, true),
        };
        for (end, many, dir) in [(pts.first().copied(), many_a, da), (pts.last().copied(), many_b, db)] {
            let Some(e) = end else { continue };
            let c = e + vec2(dir * 13.0 * z, -9.0 * z);
            painter.circle_filled(c, 7.0 * z, t.panel);
            painter.circle_stroke(c, 7.0 * z, Stroke::new(1.0, color));
            painter.text(c + vec2(0.0, if many { 1.5 * z } else { 0.0 }), Align2::CENTER_CENTER, if many { "*" } else { "1" }, FontId::proportional(if many { 13.0 } else { 10.0 } * z), color);
        }
        // Filter direction: from the one side to the many side (both ways when bidirectional).
        let towards_a = many_a || !many_b;
        let (mid, dir) = along(&pts, 0.5);
        let fwd = if towards_a { -dir } else { dir };
        if r.cross_filter == CrossFilterDir::Both {
            arrow(&painter, mid + fwd * 7.0 * z, fwd, color, z);
            arrow(&painter, mid - fwd * 7.0 * z, -fwd, color, z);
        } else {
            arrow(&painter, mid, fwd, color, z);
        }
    }
    // Cards.
    let mut card_acts: Vec<Act> = Vec::new();
    let mut moved: Option<(String, Vec2)> = None;
    let mut drag_started: Option<String> = None;
    let mut drop_rel: Option<((String, String), (String, String))> = None;
    let mut toggles: Vec<(String, bool)> = Vec::new();
    let mut remove_from_layout = None;
    for (m, list) in &its {
        let Some(g) = geoms.get(&m.name) else { continue };
        let r = g.rect;
        if !r.intersects(rect) {
            continue;
        }
        let kc = m.kind.color(t);
        let selected_table = matches!(&sel, Sel::Table(x) if x == &m.name);
        // Shadow, body.
        for (i, a) in [(6.0, 10u8), (3.0, 16u8)] {
            painter.rect_filled(r.translate(vec2(0.0, i * 0.5 * z)).expand(i * z * 0.5), 10.0 * z, Color32::from_black_alpha(if t.dark { a * 3 } else { a }));
        }
        painter.rect(r, 8.0 * z, t.panel, Stroke::new(if selected_table { 2.0 } else { 1.0 }, if selected_table { t.accent } else { t.border }), StrokeKind::Inside);
        // Header strip.
        let head = Rect::from_min_size(r.min, vec2(r.width(), HEAD_H * z));
        painter.rect_filled(Rect::from_min_size(head.min, vec2(head.width(), 4.0 * z)), egui::CornerRadius { nw: (8.0 * z) as u8, ne: (8.0 * z) as u8, sw: 0, se: 0 }, kc);
        painter.rect_filled(Rect::from_min_max(head.min + vec2(0.0, 4.0 * z), head.max), 0.0, kc.gamma_multiply(if t.dark { 0.16 } else { 0.08 }));
        painter.line_segment([head.left_bottom(), head.right_bottom()], Stroke::new(1.0, t.border));
        let name = vizcraft_core::scene::ellipsize(&m.name, 13.5, (r.width() - 46.0 * z) / z.max(0.1));
        painter.text(head.left_top() + vec2(12.0 * z, 16.0 * z), Align2::LEFT_CENTER, name, FontId::proportional(13.5 * z), if m.hidden { t.muted } else { t.text });
        painter.text(head.left_top() + vec2(12.0 * z, 32.0 * z), Align2::LEFT_CENTER, format!("{} · {} rows", m.kind.label(), fmt_count(m.rows)), FontId::proportional(10.5 * z), t.muted);
        // Collapse chevron.
        let chev = Rect::from_center_size(head.right_top() + vec2(-16.0 * z, 16.0 * z), vec2(18.0 * z, 18.0 * z));
        let chev_r = ui.interact(chev, ui.scope_id().with(("mv_chev", &m.name)), Sense::click());
        if chev_r.hovered() {
            painter.rect_filled(chev, 4.0, t.accent_soft);
        }
        tri(&painter, chev.center(), if list.is_empty() { Dir::Right } else { Dir::Down }, 4.0 * z, t.muted);
        if chev_r.clicked() {
            toggles.push((m.name.clone(), list.is_empty()));
        }
        // Header drag → move; click → select.
        let hr = ui.interact(head.with_max_x(chev.left()), ui.scope_id().with(("mv_head", &m.name)), Sense::click_and_drag());
        if hr.drag_started() {
            drag_started = Some(m.name.clone());
        }
        if hr.dragged() {
            moved = Some((m.name.clone(), hr.drag_delta() / z));
        }
        if hr.clicked() {
            card_acts.push(Act::Select(Sel::Table(m.name.clone())));
        }
        if hr.double_clicked() {
            card_acts.push(Act::ShowData(m.name.clone()));
        }
        let tname = m.name.clone();
        let (kind, hidden) = (m.kind, m.hidden);
        hr.context_menu(|ui| {
            if ui.button("New measure").clicked() {
                card_acts.push(Act::NewMeasure(tname.clone()));
                ui.close();
            }
            if ui.button("New column").clicked() {
                card_acts.push(Act::NewColumn(tname.clone()));
                ui.close();
            }
            if ui.button("Mark as date table…").clicked() {
                card_acts.push(Act::MarkDate(tname.clone()));
                ui.close();
            }
            ui.separator();
            if ui.button(if hidden { "Unhide in report view" } else { "Hide in report view" }).clicked() {
                card_acts.push(Act::ToggleHidden(Sel::Table(tname.clone())));
                ui.close();
            }
            if ui.button("View data").clicked() {
                card_acts.push(Act::ShowData(tname.clone()));
                ui.close();
            }
            if ui.button("Script TMDL to a new tab").clicked() {
                card_acts.push(Act::ScriptTmdl(vizcraft_core::tmdl::TmdlObject::Table(tname.clone())));
                ui.close();
            }
            if li > 0 && ui.button("Remove from this layout").clicked() {
                remove_from_layout = Some(tname.clone());
                ui.close();
            }
            if kind == TableKind::Calculated {
                ui.separator();
                if ui.button("Delete table").clicked() {
                    card_acts.push(Act::Delete(Sel::Table(tname.clone())));
                    ui.close();
                }
            }
        });
        // Items.
        let mut y = head.bottom();
        let clip = painter.with_clip_rect(r.intersect(rect));
        for it in list {
            let h = item_h(it) * z;
            let row = Rect::from_min_size(pos2(r.left(), y), vec2(r.width(), h));
            y += h;
            match it {
                Item::Section(label) => {
                    clip.line_segment([row.left_top() + vec2(10.0 * z, 2.0), row.right_top() - vec2(10.0 * z, -2.0)], Stroke::new(1.0, t.border));
                    clip.text(row.left_center() + vec2(12.0 * z, 2.0 * z), Align2::LEFT_CENTER, *label, FontId::proportional(9.5 * z), t.muted);
                }
                Item::Column(c) => {
                    let s = Sel::Column(m.name.clone(), c.name.clone());
                    let rr = ui.interact(row, ui.scope_id().with(("mv_col", &m.name, &c.name)), Sense::click_and_drag());
                    let target = st.rel_drag.as_ref().is_some_and(|(tt, _)| tt != &m.name) && rr.hovered();
                    if sel == s || target {
                        clip.rect_filled(row.shrink2(vec2(4.0, 1.0)), 4.0, t.accent_soft);
                    } else if rr.hovered() {
                        clip.rect_filled(row.shrink2(vec2(4.0, 1.0)), 4.0, t.panel_alt);
                    }
                    // Key columns used by relationships get a small link mark.
                    let keyed = app.project.model.relationships.iter().any(|x| (x.from_table == m.name && x.from_column == c.name) || (x.to_table == m.name && x.to_column == c.name));
                    let ic = Rect::from_center_size(row.left_center() + vec2(20.0 * z, 0.0), vec2(16.0 * z, 14.0 * z));
                    paint_type_icon(&clip, ic, c.ty, c.calc, c.summarize, if c.hidden { t.muted } else { kc });
                    let label = vizcraft_core::scene::ellipsize(&c.name, 12.0, (r.width() - 70.0 * z) / z.max(0.1));
                    clip.text(row.left_center() + vec2(34.0 * z, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(12.0 * z), if c.hidden { t.muted } else { t.text });
                    if keyed {
                        let k = row.right_center() - vec2(14.0 * z, 0.0);
                        clip.circle_stroke(k - vec2(3.0 * z, 0.0), 3.0 * z, Stroke::new(1.2, t.muted));
                        clip.circle_stroke(k + vec2(3.0 * z, 0.0), 3.0 * z, Stroke::new(1.2, t.muted));
                    }
                    if c.hidden {
                        let k = row.right_center() - vec2(if keyed { 30.0 } else { 14.0 } * z, 0.0);
                        clip.line_segment([k + vec2(-5.0 * z, 3.0 * z), k + vec2(5.0 * z, -3.0 * z)], Stroke::new(1.0, t.muted));
                        clip.circle_stroke(k, 3.0 * z, Stroke::new(1.0, t.muted));
                    }
                    if rr.clicked() {
                        card_acts.push(Act::Select(s.clone()));
                    }
                    if rr.drag_started() {
                        st.rel_drag = Some((m.name.clone(), c.name.clone()));
                    }
                    let (tn, cn, calc, hid) = (m.name.clone(), c.name.clone(), c.calc, c.hidden);
                    rr.context_menu(|ui| {
                        if calc && ui.button("Edit column formula…").clicked() {
                            card_acts.push(Act::EditColumn(tn.clone(), cn.clone()));
                            ui.close();
                        }
                        if ui.button(if hid { "Unhide in report view" } else { "Hide in report view" }).clicked() {
                            card_acts.push(Act::ToggleHidden(Sel::Column(tn.clone(), cn.clone())));
                            ui.close();
                        }
                        if ui.button("Show values (DAX query)").clicked() {
                            card_acts.push(Act::ColumnValues(tn.clone(), cn.clone()));
                            ui.close();
                        }
                        if calc && ui.button("Delete column").clicked() {
                            card_acts.push(Act::Delete(Sel::Column(tn.clone(), cn.clone())));
                            ui.close();
                        }
                    });
                }
                Item::Measure(n, hidden) => {
                    let s = Sel::Measure(n.clone());
                    let rr = ui.interact(row, ui.scope_id().with(("mv_m", n)), Sense::click());
                    if sel == s {
                        clip.rect_filled(row.shrink2(vec2(4.0, 1.0)), 4.0, t.accent_soft);
                    } else if rr.hovered() {
                        clip.rect_filled(row.shrink2(vec2(4.0, 1.0)), 4.0, t.panel_alt);
                    }
                    crate::icons::paint(&clip, Rect::from_center_size(row.left_center() + vec2(20.0 * z, 0.0), vec2(14.0 * z, 14.0 * z)), "measure", if *hidden { t.muted } else { t.good });
                    clip.text(row.left_center() + vec2(34.0 * z, 0.0), Align2::LEFT_CENTER, vizcraft_core::scene::ellipsize(n, 12.0, (r.width() - 50.0 * z) / z.max(0.1)), FontId::proportional(12.0 * z), if *hidden { t.muted } else { t.text });
                    if rr.clicked() {
                        card_acts.push(Act::Select(s));
                    }
                    let user = app.project.model.measures.iter().any(|x| &x.name == n);
                    if rr.double_clicked() && user {
                        card_acts.push(Act::EditMeasure(n.clone()));
                    }
                    let n2 = n.clone();
                    rr.context_menu(|ui| {
                        if user && ui.button("Edit measure…").clicked() {
                            card_acts.push(Act::EditMeasure(n2.clone()));
                            ui.close();
                        }
                        if user && ui.button("Define and evaluate (DAX query)").clicked() {
                            card_acts.push(Act::DefineEvaluate(n2.clone()));
                            ui.close();
                        }
                        if user && ui.button("Delete measure").clicked() {
                            card_acts.push(Act::Delete(Sel::Measure(n2.clone())));
                            ui.close();
                        }
                    });
                }
                Item::Hierarchy(n, levels) => {
                    let ic = row.left_center() + vec2(14.0 * z, -5.0 * z);
                    for i in 0..3 {
                        clip.rect_filled(Rect::from_min_size(ic + vec2(i as f32 * 3.0 * z, i as f32 * 4.0 * z), vec2(6.0 * z, 3.0 * z)), 1.0, kc);
                    }
                    clip.text(row.left_center() + vec2(34.0 * z, 0.0), Align2::LEFT_CENTER, n, FontId::proportional(12.0 * z), t.text);
                    clip.text(row.right_center() - vec2(12.0 * z, 0.0), Align2::RIGHT_CENTER, format!("{levels} levels"), FontId::proportional(10.0 * z), t.muted);
                }
                Item::More(k) => {
                    let rr = ui.interact(row, ui.scope_id().with(("mv_more", &m.name)), Sense::click());
                    clip.text(row.left_center() + vec2(34.0 * z, 0.0), Align2::LEFT_CENTER, format!("Show {k} more"), FontId::proportional(11.5 * z), t.accent);
                    if rr.clicked() {
                        toggles.push((m.name.clone(), true));
                        st.expanded.insert(m.name.clone());
                    }
                }
            }
        }
    }
    // Relationship drag: a line from the source column to the pointer; release on a column of
    // another table to create the relationship.
    if let Some((ft, fc)) = st.rel_drag.clone() {
        let from = geoms.get(&ft).and_then(|g| g.rows.get(&fc).map(|r| r.right_center()));
        if let (Some(a), Some(p)) = (from, pointer) {
            painter.add(Shape::dashed_line(&[a, p], Stroke::new(2.0, t.accent), 6.0, 4.0));
            painter.circle_filled(p, 4.0, t.accent);
        }
        if ui.input(|i| i.pointer.any_released()) {
            if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                for (tn, g) in &geoms {
                    if *tn == ft {
                        continue;
                    }
                    if let Some((cn, _)) = g.rows.iter().find(|(_, r)| r.contains(p)) {
                        drop_rel = Some(((ft.clone(), fc.clone()), (tn.clone(), cn.clone())));
                    }
                }
            }
            st.rel_drag = None;
        }
    }
    // Background: pan by dragging, scroll to pan, Ctrl+scroll to zoom, click to select a line.
    if bg.dragged() && st.drag_card.is_none() && st.rel_drag.is_none() {
        st.pan += bg.drag_delta();
    }
    if ui.rect_contains_pointer(rect) {
        let (zoom, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
        if (zoom - 1.0).abs() > 1e-4 {
            let nz = (z * zoom).clamp(0.3, 2.0);
            if let Some(p) = pointer {
                // Keep the point under the cursor in place.
                let world_p = (p - rect.min - st.pan) / z;
                st.pan = p - rect.min - world_p * nz;
            }
            z = nz;
            if let Some(l) = app.project.model.layouts.get_mut(li) {
                l.zoom = nz;
            }
        } else if scroll != Vec2::ZERO {
            st.pan += scroll;
        }
    }
    if bg.clicked() {
        match hit_rel {
            Some((id, _)) => card_acts.push(Act::Select(Sel::Relationship(id))),
            None => card_acts.push(Act::Select(Sel::None)),
        }
    }
    if bg.double_clicked()
        && let Some((id, _)) = hit_rel
    {
        card_acts.push(Act::EditRelationship(id));
    }
    if hit_rel.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        if let Some((id, _)) = hit_rel
            && let Some(r) = app.project.model.relationships.iter().find(|r| r.id == id)
        {
            bg.clone().on_hover_text_at_pointer(format!("{}[{}] → {}[{}]\n{} · {}", r.from_table, r.from_column, r.to_table, r.to_column, r.cardinality.label(), if r.active { "active" } else { "inactive" }));
        }
    }
    let _ = z;
    // Empty custom layout.
    if its.is_empty() && li > 0 {
        let c = rect.center();
        painter.text(c - vec2(0.0, 30.0), Align2::CENTER_CENTER, "This layout is empty", FontId::proportional(16.0), t.text);
        painter.text(c - vec2(0.0, 8.0), Align2::CENTER_CENTER, "Add tables to it:", FontId::proportional(12.0), t.muted);
        let names: Vec<String> = metas.iter().map(|m| m.name.clone()).collect();
        let mut add = None;
        let area = Rect::from_center_size(c + vec2(0.0, 40.0), vec2(rect.width().min(640.0), 60.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true)));
        if child.button("Add all tables").clicked() {
            add = Some(names.clone());
        }
        for n in &names {
            if child.button(n).clicked() {
                add = Some(vec![n.clone()]);
            }
        }
        if let Some(list) = add {
            app.checkpoint("Add tables to layout");
            let all_pos = app.project.model.layouts.first().map(|l| l.positions.clone()).unwrap_or_default();
            if let Some(l) = app.project.model.layouts.get_mut(li) {
                for n in list {
                    if !l.tables.contains(&n) {
                        l.tables.push(n.clone());
                    }
                    let p = all_pos.get(&n).copied().unwrap_or((40.0, 40.0));
                    l.positions.entry(n).or_insert(p);
                }
            }
            app.model_ui.diagram.fit = true;
        }
    }
    // Apply.
    if let Some(name) = drag_started {
        app.checkpoint("Move table");
        app.model_ui.diagram.drag_card = Some(name);
    }
    if let Some((name, d)) = moved
        && let Some(l) = app.project.model.layouts.get_mut(li)
    {
        let p = l.positions.entry(name).or_insert((40.0, 40.0));
        p.0 += d.x;
        p.1 += d.y;
    }
    if ui.input(|i| i.pointer.any_released()) {
        app.model_ui.diagram.drag_card = None;
    }
    for (name, open) in toggles {
        let st = &mut app.model_ui.diagram;
        if open {
            st.collapsed.remove(&name);
        } else {
            st.collapsed.insert(name.clone());
            st.expanded.remove(&name);
        }
    }
    if let Some(n) = remove_from_layout {
        app.checkpoint("Remove from layout");
        if let Some(l) = app.project.model.layouts.get_mut(li) {
            l.tables.retain(|x| x != &n);
        }
    }
    if let Some((a, b)) = drop_rel {
        create_relationship(app, a, b);
    }
    for a in card_acts {
        run_act(app, a);
    }
    // Delete key removes the selected relationship.
    if let Sel::Relationship(id) = app.model_ui.sel
        && !ui.ctx().egui_wants_keyboard_input()
        && ui.input(|i| i.key_pressed(egui::Key::Delete))
    {
        super::delete(app, &Sel::Relationship(id));
    }
}

/// Creates a relationship between two columns dropped on each other; the data decides which
/// side is the one side.
pub fn create_relationship(app: &mut VizApp, a: (String, String), b: (String, String)) {
    let unique = |t: &str, c: &str| super::dialogs::is_unique(&app.built, t, c);
    let (ua, ub) = (unique(&a.0, &a.1), unique(&b.0, &b.1));
    let (from, to, card) = match (ua, ub) {
        (_, true) if !ua => (a.clone(), b.clone(), Cardinality::ManyToOne),
        (true, false) => (b.clone(), a.clone(), Cardinality::ManyToOne),
        (true, true) => (a.clone(), b.clone(), Cardinality::OneToOne),
        _ => (a.clone(), b.clone(), Cardinality::ManyToMany),
    };
    if app.project.model.relationships.iter().any(|r| (r.from_table == from.0 && r.from_column == from.1 && r.to_table == to.0 && r.to_column == to.1) || (r.from_table == to.0 && r.from_column == to.1 && r.to_table == from.0 && r.to_column == from.1)) {
        app.error("Those columns are already related.");
        return;
    }
    // Only one active path between two tables.
    let active = !app.project.model.relationships.iter().any(|r| r.active && ((r.from_table == from.0 && r.to_table == to.0) || (r.from_table == to.0 && r.to_table == from.0)));
    app.checkpoint("New relationship");
    let id = app.project.next_id();
    app.project.model.relationships.push(Relationship {
        id,
        from_table: from.0.clone(),
        from_column: from.1.clone(),
        to_table: to.0.clone(),
        to_column: to.1.clone(),
        cardinality: card,
        cross_filter: if card == Cardinality::OneToOne { CrossFilterDir::Both } else { CrossFilterDir::Single },
        active,
        ..Relationship::default()
    });
    app.model_ui.sel = Sel::Relationship(id);
    app.mark_model_dirty();
    let note = match card {
        Cardinality::ManyToMany => " (many-to-many: neither column has unique values)",
        _ if !active => " (inactive: the tables already have an active relationship)",
        _ => "",
    };
    app.toast(format!("Related {}[{}] → {}[{}] · {}{note}", from.0, from.1, to.0, to.1, card.label()));
}

// ------------------------------------------------------------------------------------- properties

fn properties(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    ui.label(egui::RichText::new("Properties").strong().size(14.0));
    ui.add_space(6.0);
    let sel = app.model_ui.sel.clone();
    egui::ScrollArea::vertical().id_salt("mv_props_scroll").auto_shrink([false, false]).show(ui, |ui| match &sel {
        Sel::Relationship(id) => relationship_props(app, ui, t, *id),
        Sel::None => {
            ui.add_space(10.0);
            ui.label(egui::RichText::new("Select a table, column, measure or relationship to see and edit its properties.").color(t.muted));
            ui.add_space(12.0);
            let rels = app.project.model.relationships.len();
            let metas = tables_meta(&app.project, &app.built);
            ui.label(egui::RichText::new("Model").strong());
            for (k, v) in [
                ("Tables", metas.len().to_string()),
                ("Relationships", rels.to_string()),
                ("Measures", app.project.model.measures.len().to_string()),
                ("Calculated columns", app.project.model.calculated_columns.len().to_string()),
                ("Roles", app.project.model.roles.len().to_string()),
                ("Rows", fmt_count(metas.iter().map(|m| m.rows).sum())),
            ] {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(k).color(t.muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(v));
                });
            }
            ui.add_space(10.0);
            let mut auto = app.project.model.auto_date_time;
            if ui.checkbox(&mut auto, "Auto date/time").on_hover_text("Year / Quarter / Month / Day levels for every date column").changed() {
                app.checkpoint("Auto date/time");
                app.project.model.auto_date_time = auto;
                app.mark_model_dirty();
            }
        }
        _ => super::table_view::properties_pane(app, ui, t),
    });
}

fn relationship_props(app: &mut VizApp, ui: &mut Ui, t: &Tokens, id: u64) {
    let Some(r) = app.project.model.relationships.iter().find(|r| r.id == id).cloned() else { return };
    ui.label(egui::RichText::new("Relationship").strong().size(13.0));
    ui.add_space(4.0);
    for (k, v) in [("From", format!("{}[{}]", r.from_table, r.from_column)), ("To", format!("{}[{}]", r.to_table, r.to_column))] {
        ui.horizontal(|ui| {
            ui.add_sized([60.0, 18.0], egui::Label::new(egui::RichText::new(k).color(t.muted)));
            ui.label(egui::RichText::new(v).monospace());
        });
    }
    ui.add_space(6.0);
    let mut card = r.cardinality;
    let mut cross = r.cross_filter;
    let mut active = r.active;
    let mut sec = r.security_both;
    let mut rely = r.assume_integrity;
    ui.label(egui::RichText::new("Cardinality").size(11.5).color(t.muted));
    egui::ComboBox::from_id_salt("mv_rel_card").selected_text(card.label()).width(220.0).show_ui(ui, |ui| {
        for c in Cardinality::ALL {
            ui.selectable_value(&mut card, c, c.label());
        }
    });
    ui.label(egui::RichText::new("Cross-filter direction").size(11.5).color(t.muted));
    egui::ComboBox::from_id_salt("mv_rel_cross").selected_text(if cross == CrossFilterDir::Both { "Both" } else { "Single" }).width(220.0).show_ui(ui, |ui| {
        ui.selectable_value(&mut cross, CrossFilterDir::Single, "Single");
        ui.selectable_value(&mut cross, CrossFilterDir::Both, "Both");
    });
    ui.checkbox(&mut active, "Make this relationship active");
    ui.add_enabled(cross == CrossFilterDir::Both, egui::Checkbox::new(&mut sec, "Apply security filter in both directions"));
    ui.checkbox(&mut rely, "Assume referential integrity");
    if (card, cross, active, sec, rely) != (r.cardinality, r.cross_filter, r.active, r.security_both, r.assume_integrity) {
        app.checkpoint("Edit relationship");
        if let Some(x) = app.project.model.relationships.iter_mut().find(|x| x.id == id) {
            x.cardinality = card;
            x.cross_filter = cross;
            x.active = active;
            x.security_both = sec;
            x.assume_integrity = rely;
        }
        app.mark_model_dirty();
    }
    ui.add_space(8.0);
    for (k, msg) in super::dialogs::validate_relationship(&app.project, &app.built, &r) {
        ui.horizontal_wrapped(|ui| {
            let (rr, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            glyph_icon(ui.painter(), rr, if k { "check" } else { "warn" }, if k { t.good } else { Color32::from_rgb(0xD9, 0x8A, 0x0B) });
            ui.label(egui::RichText::new(msg).size(11.5));
        });
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button("Edit…").clicked() {
            super::dialogs::edit_relationship(app, id);
        }
        if ui.button("Delete").clicked() {
            super::delete(app, &Sel::Relationship(id));
        }
    });
}
