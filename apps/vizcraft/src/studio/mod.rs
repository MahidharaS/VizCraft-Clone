//! Data Science Studio: an Orange-style canvas for machine learning and statistics, with MLOps.
//!
//! Left: widget palette by category. Centre: the workflow canvas (drag widgets, drag from an
//! output port to an input port to connect). Right: the selected widget's settings. Bottom: the
//! widget's window (data table, report, plot, scores…). Deploy widgets register models in
//! `~/.vizcraft/models`, which `vizcraft-mlserve` (or the built-in server here) serves over HTTP.

mod plots;

use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Value as J, json};
use vizcraft_core::build::TableData;
use vizcraft_core::studio::exec::{self, Ctx, Executor, Out, State, View};
use vizcraft_core::studio::{self, Category, Chan, ColKind, ParamKind, StudioWorkflow, WidgetType, eval, examples, registry};

use crate::app::VizApp;
use crate::theme::Tokens;

const NODE_W: f32 = 150.0;
const NODE_H: f32 = 58.0;

pub struct StudioUi {
    pub current: usize,
    pub selected: Option<u64>,
    pub executor: Executor,
    pub pan: egui::Vec2,
    pub zoom: f32,
    fit: bool,
    shown: Option<(usize, usize)>,
    pub search: String,
    connecting: Option<(u64, usize)>,
    dragging: Option<u64>,
    pub message: Option<(String, bool)>,
    pub server: Option<vizcraft_mlops::server::Handle>,
    pub port: u16,
    env_checked: bool,
}

impl Default for StudioUi {
    fn default() -> Self {
        StudioUi { current: 0, selected: None, executor: Executor::default(), pan: vec2(20.0, 20.0), zoom: 1.0, fit: true, shown: None, search: String::new(), connecting: None, dragging: None, message: None, server: None, port: 8765, env_checked: false }
    }
}

fn cat_color(c: Category) -> Color32 {
    let (r, g, b) = c.rgb();
    Color32::from_rgb(r, g, b)
}

fn chan_color(c: Chan) -> Color32 {
    match c {
        Chan::Data => Color32::from_rgb(0x3B, 0x82, 0xF6),
        Chan::Model => Color32::from_rgb(0xEA, 0x8C, 0x0C),
        Chan::Eval => Color32::from_rgb(0xDB, 0x27, 0x77),
    }
}

/// Adds an example workflow (or an empty one) and selects it.
pub fn new_workflow(app: &mut VizApp, example: Option<&str>) {
    let wf = example.and_then(examples::build).unwrap_or_else(|| StudioWorkflow { name: format!("Workflow {}", app.project.studio.len() + 1), ..Default::default() });
    app.project.studio.push(wf);
    app.studio_ui.current = app.project.studio.len() - 1;
    app.studio_ui.selected = None;
    app.studio_ui.pan = vec2(20.0, 20.0);
    app.studio_ui.fit = true;
}

fn run(app: &VizApp, st: &mut StudioUi) {
    let Some(wf) = app.project.studio.get(st.current) else { return };
    let base_dir = app.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let ctx = Ctx { tables: &app.built.tables, generation: app.generation, registry: &app.project.models, base_dir };
    st.executor.run(&ctx, wf);
}

pub fn show(app: &mut VizApp, ui: &mut Ui) {
    let mut st = std::mem::take(&mut app.studio_ui);
    let t = app.tokens();
    if !st.env_checked {
        st.env_checked = true;
        if let Ok(v) = std::env::var("VIZCRAFT_OPEN")
            && let Some(key) = v.strip_prefix("studio-")
        {
            new_workflow(app, Some(key));
            st.current = app.studio_ui.current;
            st.selected = app.project.studio.get(st.current).and_then(|w| w.widgets.last().map(|w| w.id));
        }
    }
    if st.current >= app.project.studio.len() {
        st.current = app.project.studio.len().saturating_sub(1);
    }
    run(app, &mut st);

    let pane = egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::same(8)).stroke(Stroke::new(1.0, t.border));
    egui::Panel::top("studio-tabs").frame(egui::Frame::new().fill(t.panel_alt).inner_margin(egui::Margin::symmetric(8, 5))).show(ui, |ui| tabs(app, &mut st, ui, &t));
    if app.project.studio.is_empty() {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ui, |ui| empty(app, &mut st, ui, &t));
        app.studio_ui = st;
        return;
    }
    egui::Panel::left("studio-palette").default_size(190.0).size_range(160.0..=280.0).frame(pane).show(ui, |ui| palette(app, &mut st, ui, &t));
    egui::Panel::right("studio-inspector").default_size(270.0).size_range(220.0..=420.0).frame(pane).show(ui, |ui| inspector(app, &mut st, ui, &t));
    egui::Panel::bottom("studio-window").default_size(330.0).size_range(140.0..=700.0).resizable(true).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::same(10)).stroke(Stroke::new(1.0, t.border))).show(ui, |ui| window(app, &mut st, ui, &t));
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.canvas)).show(ui, |ui| canvas(app, &mut st, ui, &t));
    app.studio_ui = st;
}

fn tabs(app: &mut VizApp, st: &mut StudioUi, ui: &mut Ui, t: &Tokens) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Data Science Studio").strong().size(13.0));
        ui.add_space(10.0);
        let mut close = None;
        for (i, wf) in app.project.studio.iter().enumerate() {
            let sel = i == st.current;
            let r = ui.add(egui::Button::selectable(sel, RichText::new(&wf.name).size(12.0)));
            if r.clicked() {
                st.current = i;
                st.selected = None;
            }
            r.context_menu(|ui| {
                if ui.button("Close workflow").clicked() {
                    close = Some(i);
                    ui.close();
                }
            });
        }
        if let Some(i) = close {
            app.project.studio.remove(i);
            st.current = st.current.min(app.project.studio.len().saturating_sub(1));
            st.selected = None;
            st.executor.clear();
        }
        ui.menu_button("+ New", |ui| {
            if ui.button("Empty workflow").clicked() {
                new_workflow(app, None);
                st.current = app.project.studio.len() - 1;
                ui.close();
            }
            ui.separator();
            ui.label(RichText::new("Examples (Retail sample)").size(11.0).color(t.muted));
            for (k, name) in examples::EXAMPLES {
                if ui.button(name).clicked() {
                    new_workflow(app, Some(k));
                    st.current = app.project.studio.len() - 1;
                    st.selected = None;
                    ui.close();
                }
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            server_controls(app, st, ui, t);
            ui.separator();
            if ui.button("Fit").on_hover_text("Fit the workflow in view (Ctrl + wheel zooms)").clicked() {
                st.fit = true;
            }
            ui.label(RichText::new(format!("{:.0}%", st.zoom * 100.0)).size(11.0).color(t.muted));
            if let Some((m, err)) = &st.message {
                ui.label(RichText::new(m).size(11.0).color(if *err { t.danger } else { t.good }));
            }
        });
    });
}

fn server_controls(app: &mut VizApp, st: &mut StudioUi, ui: &mut Ui, t: &Tokens) {
    let running = st.server.as_ref().is_some_and(|h| h.running());
    if running {
        if ui.button("Stop server").clicked() {
            st.server = None;
            st.message = Some(("Model server stopped".into(), false));
        }
        let addr = st.server.as_ref().map(|h| h.addr.clone()).unwrap_or_default();
        ui.label(RichText::new(format!("● serving on http://{addr}")).size(11.0).color(t.good));
    } else {
        if ui.button("Serve models").on_hover_text("Serve registered models over HTTP: POST /models/<name>/predict").clicked() {
            let reg = vizcraft_mlops::Registry::open_default();
            let _ = reg.sync(&app.project.models);
            match vizcraft_mlops::server::start(reg, "127.0.0.1", st.port) {
                Ok(h) => {
                    st.message = Some((format!("Serving {} model version(s)", app.project.models.len()), false));
                    st.server = Some(h);
                }
                Err(e) => st.message = Some((e, true)),
            }
        }
        ui.add(egui::DragValue::new(&mut st.port).range(1024..=65535).prefix("port "));
    }
    ui.label(RichText::new(format!("{} registered", app.project.models.len())).size(11.0).color(t.muted));
}

fn empty(app: &mut VizApp, st: &mut StudioUi, ui: &mut Ui, t: &Tokens) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.22);
        ui.label(RichText::new("Data Science Studio").size(22.0).strong());
        ui.label(RichText::new("Build machine-learning and statistical models by connecting widgets: data → transform → model → evaluate → deploy.").color(t.muted));
        ui.add_space(16.0);
        if ui.add(egui::Button::new(RichText::new("New empty workflow").size(13.0)).min_size(vec2(260.0, 30.0))).clicked() {
            new_workflow(app, None);
            st.current = app.project.studio.len() - 1;
        }
        ui.add_space(10.0);
        ui.label(RichText::new("or start from an example on the Retail sample").size(11.5).color(t.muted));
        for (k, name) in examples::EXAMPLES {
            if ui.add(egui::Button::new(name).min_size(vec2(260.0, 28.0))).clicked() {
                new_workflow(app, Some(k));
                st.current = app.project.studio.len() - 1;
            }
        }
    });
}

// =============================================================================================
// Palette
// =============================================================================================

const CATEGORIES: [Category; 8] = [Category::Data, Category::Transform, Category::Model, Category::Evaluate, Category::Visualize, Category::Unsupervised, Category::Statistics, Category::Deploy];

fn palette(app: &mut VizApp, st: &mut StudioUi, ui: &mut Ui, _t: &Tokens) {
    ui.add(egui::TextEdit::singleline(&mut st.search).hint_text("Search widgets").desired_width(f32::INFINITY));
    ui.add_space(4.0);
    let catalog = studio::catalog();
    let q = st.search.to_lowercase();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for cat in CATEGORIES {
            let items: Vec<&WidgetType> = catalog.iter().filter(|w| w.category == cat && (q.is_empty() || w.name.to_lowercase().contains(&q) || w.description.to_lowercase().contains(&q))).collect();
            if items.is_empty() {
                continue;
            }
            egui::CollapsingHeader::new(RichText::new(cat.label()).strong().size(12.0).color(cat_color(cat))).id_salt(("st-cat", cat.label())).default_open(!q.is_empty() || matches!(cat, Category::Data | Category::Model | Category::Evaluate)).show(ui, |ui| {
                for w in items {
                    let r = ui.add(egui::Button::new(RichText::new(w.name).size(11.5)).frame(false)).on_hover_text(w.description);
                    if r.clicked() {
                        add_widget(app, st, w.id);
                    }
                }
            });
        }
    });
}

/// Adds a widget next to the selection and connects it when the channels fit.
fn add_widget(app: &mut VizApp, st: &mut StudioUi, kind: &str) {
    let Some(wf) = app.project.studio.get_mut(st.current) else { return };
    let (x, y, from) = match st.selected.and_then(|s| wf.widget(s)) {
        Some(s) => (s.x + NODE_W + 50.0, s.y, Some(s.id)),
        None => (40.0 + (wf.widgets.len() % 5) as f32 * 40.0, 40.0 + (wf.widgets.len() % 7) as f32 * 70.0, None),
    };
    let id = wf.add(kind, x, y);
    if let Some(from) = from
        && let (Some(a), Some(b)) = (wf.widget(from).and_then(|w| studio::widget_type(&w.kind)), studio::widget_type(kind))
        && let Some((op, ip)) = a.outputs.iter().enumerate().find_map(|(oi, o)| b.inputs.iter().position(|i| o.chan.fits(i.chan)).map(|ii| (oi, ii)))
    {
        let _ = wf.connect(from, op, id, ip);
    }
    st.selected = Some(id);
}

// =============================================================================================
// Canvas
// =============================================================================================

fn port_pos(w: &studio::StudioWidget, n: usize, i: usize, output: bool, origin: Pos2, z: f32) -> Pos2 {
    let y = w.y + NODE_H * (i as f32 + 1.0) / (n as f32 + 1.0);
    origin + vec2(if output { w.x + NODE_W } else { w.x }, y) * z
}

fn bezier(p: &egui::Painter, a: Pos2, b: Pos2, stroke: Stroke) {
    let dx = ((b.x - a.x).abs() * 0.5).max(40.0);
    let pts = [a, a + vec2(dx, 0.0), b - vec2(dx, 0.0), b];
    p.add(egui::epaint::CubicBezierShape::from_points_stroke(pts, false, Color32::TRANSPARENT, stroke));
}

fn canvas(app: &mut VizApp, st: &mut StudioUi, ui: &mut Ui, t: &Tokens) {
    let (rect, bg) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let p = ui.painter_at(rect);
    // Dot grid.
    let step = 24.0;
    let mut gx = rect.left() + st.pan.x.rem_euclid(step);
    while gx < rect.right() {
        let mut gy = rect.top() + st.pan.y.rem_euclid(step);
        while gy < rect.bottom() {
            p.circle_filled(pos2(gx, gy), 0.9, t.border);
            gy += step;
        }
        gx += step;
    }
    let Some(wf) = app.project.studio.get(st.current).cloned() else { return };
    // Fit the workflow into view when it is opened (and on request).
    let key = (st.current, app.project.studio.len());
    if st.shown != Some(key) {
        st.shown = Some(key);
        st.fit = true;
    }
    if st.fit && !wf.widgets.is_empty() {
        st.fit = false;
        let (mut lo, mut hi) = (pos2(f32::MAX, f32::MAX), pos2(f32::MIN, f32::MIN));
        for w in &wf.widgets {
            lo = lo.min(pos2(w.x, w.y));
            hi = hi.max(pos2(w.x + NODE_W, w.y + NODE_H));
        }
        let span = (hi - lo) + vec2(80.0, 80.0);
        st.zoom = (rect.width() / span.x).min(rect.height() / span.y).clamp(0.4, 1.0);
        st.pan = (rect.size() - (hi - lo) * st.zoom) / 2.0 - lo.to_vec2() * st.zoom;
    }
    // Ctrl/⌘ + wheel (or pinch) zooms around the pointer.
    if let Some(hp) = bg.hover_pos() {
        let zd = ui.input(|i| i.zoom_delta());
        if zd != 1.0 {
            let nz = (st.zoom * zd).clamp(0.3, 2.0);
            let world = (hp - rect.min - st.pan) / st.zoom;
            st.pan = hp - rect.min - world * nz;
            st.zoom = nz;
        }
    }
    let z = st.zoom;
    let origin = rect.min + st.pan;
    if bg.dragged() && st.dragging.is_none() && st.connecting.is_none() {
        st.pan += bg.drag_delta();
    }
    if bg.clicked() {
        st.selected = None;
    }
    if wf.widgets.is_empty() {
        p.text(rect.center(), Align2::CENTER_CENTER, "Click widgets in the palette to add them; drag from a port on the right of a widget to a port on the left of another to connect.", FontId::proportional(13.0), t.muted);
    }
    // Links.
    for l in &wf.links {
        let (Some(a), Some(b)) = (wf.widget(l.from), wf.widget(l.to)) else { continue };
        let (Some(ta), Some(tb)) = (studio::widget_type(&a.kind), studio::widget_type(&b.kind)) else { continue };
        let pa = port_pos(a, ta.outputs.len(), l.from_port, true, origin, z);
        let pb = port_pos(b, tb.inputs.len(), l.to_port, false, origin, z);
        let c = ta.outputs.get(l.from_port).map(|o| chan_color(o.chan)).unwrap_or(t.muted);
        bezier(&p, pa, pb, Stroke::new(2.0, c.gamma_multiply(0.8)));
        let mid = pos2((pa.x + pb.x) / 2.0, (pa.y + pb.y) / 2.0);
        let label = ta.outputs.get(l.from_port).map(|o| o.name).unwrap_or("");
        p.text(mid - vec2(0.0, 7.0), Align2::CENTER_BOTTOM, label, FontId::proportional(9.0 * z), t.muted);
    }
    let pointer = ui.ctx().pointer_interact_pos();
    let mut delete_link = None;
    let mut delete_widget = None;
    let mut new_link = None;
    for w in &wf.widgets {
        let Some(ty) = studio::widget_type(&w.kind) else { continue };
        let r = Rect::from_min_size(origin + vec2(w.x, w.y) * z, vec2(NODE_W, NODE_H) * z);
        let resp = ui.interact(r, ui.scope_id().with(("st-node", w.id)), Sense::click_and_drag());
        if resp.drag_started() {
            st.dragging = Some(w.id);
            st.selected = Some(w.id);
        }
        if resp.clicked() {
            st.selected = Some(w.id);
        }
        if resp.dragged() && st.dragging == Some(w.id) {
            let d = resp.drag_delta();
            if let Some(m) = app.project.studio.get_mut(st.current).and_then(|f| f.widget_mut(w.id)) {
                m.x += d.x / z;
                m.y += d.y / z;
            }
        }
        if resp.drag_stopped() {
            st.dragging = None;
        }
        resp.context_menu(|ui| {
            if ui.button("Delete widget").clicked() {
                delete_widget = Some(w.id);
                ui.close();
            }
            let ins: Vec<_> = wf.inputs_of(w.id).into_iter().cloned().collect();
            for l in ins {
                let from = wf.widget(l.from).map(|x| x.label.clone()).unwrap_or_default();
                if ui.button(format!("Disconnect from {from}")).clicked() {
                    delete_link = Some(l);
                    ui.close();
                }
            }
        });
        let out = st.executor.get(w.id);
        let sel = st.selected == Some(w.id);
        let cc = cat_color(ty.category);
        p.rect_filled(r, 8.0, t.panel);
        p.rect_filled(Rect::from_min_size(r.min, vec2(6.0 * z, NODE_H * z)), egui::CornerRadius { nw: 8, sw: 8, ne: 0, se: 0 }, cc);
        p.rect_stroke(r, 8.0, Stroke::new(if sel { 2.0 } else { 1.0 }, if sel { t.accent } else { t.border }), StrokeKind::Inside);
        p.text(r.min + vec2(14.0, 8.0) * z, Align2::LEFT_TOP, &w.label, FontId::proportional(12.5 * z), t.text);
        let (dot, summary) = match out.as_deref() {
            Some(Out { state: State::Ok, summary, .. }) => (t.good, summary.clone()),
            Some(Out { state: State::Warning, message, .. }) => (Color32::from_rgb(0xD9, 0x77, 0x06), message.clone()),
            Some(Out { state: State::Error, message, .. }) => (t.danger, message.clone()),
            Some(Out { message, .. }) => (t.muted, message.clone()),
            None => (t.muted, String::new()),
        };
        p.circle_filled(pos2(r.right() - 10.0 * z, r.top() + 12.0 * z), 4.0 * z, dot);
        let summary: String = summary.chars().take(28).collect();
        p.text(r.min + vec2(14.0, 28.0) * z, Align2::LEFT_TOP, summary, FontId::proportional(10.0 * z), t.muted);
        p.text(r.min + vec2(14.0, 42.0) * z, Align2::LEFT_TOP, ty.category.label(), FontId::proportional(9.0 * z), cc);
        // Ports.
        for (i, ip) in ty.inputs.iter().enumerate() {
            let pp = port_pos(w, ty.inputs.len(), i, false, origin, z);
            p.circle_filled(pp, 5.0 * z.max(0.6), chan_color(ip.chan));
            let hit = ui.interact(Rect::from_center_size(pp, vec2(14.0, 14.0)), ui.scope_id().with(("st-in", w.id, i)), Sense::hover());
            if hit.hovered() {
                p.text(pp - vec2(8.0, 0.0), Align2::RIGHT_CENTER, format!("{} ({})", ip.name, ip.chan.label()), FontId::proportional(10.0), t.text);
            }
            if let Some((fid, fp)) = st.connecting
                && ui.input(|i| i.pointer.any_released())
                && pointer.is_some_and(|q| q.distance(pp) < 12.0)
            {
                new_link = Some((fid, fp, w.id, i));
            }
        }
        for (i, op) in ty.outputs.iter().enumerate() {
            let pp = port_pos(w, ty.outputs.len(), i, true, origin, z);
            p.circle_filled(pp, 5.0 * z.max(0.6), chan_color(op.chan));
            let hit = ui.interact(Rect::from_center_size(pp, vec2(16.0, 16.0)), ui.scope_id().with(("st-out", w.id, i)), Sense::drag());
            if hit.hovered() {
                p.text(pp + vec2(8.0, 0.0), Align2::LEFT_CENTER, format!("{} ({})", op.name, op.chan.label()), FontId::proportional(10.0), t.text);
            }
            if hit.drag_started() {
                st.connecting = Some((w.id, i));
            }
            if st.connecting == Some((w.id, i))
                && let Some(q) = pointer
            {
                bezier(&p, pp, q, Stroke::new(2.0, chan_color(op.chan)));
            }
        }
    }
    if ui.input(|i| i.pointer.any_released()) {
        st.connecting = None;
    }
    if let Some(wf) = app.project.studio.get_mut(st.current) {
        if let Some((a, ap, b, bp)) = new_link {
            match wf.connect(a, ap, b, bp) {
                Ok(()) => st.message = None,
                Err(e) => st.message = Some((e, true)),
            }
        }
        if let Some(l) = delete_link {
            wf.links.retain(|x| *x != l);
        }
        if let Some(id) = delete_widget {
            wf.remove(id);
            if st.selected == Some(id) {
                st.selected = None;
            }
        }
        if let Some(s) = st.selected
            && ui.input(|i| i.key_pressed(egui::Key::Delete))
            && !ui.ctx().egui_wants_keyboard_input()
        {
            wf.remove(s);
            st.selected = None;
        }
    }
}

// =============================================================================================
// Inspector (widget settings)
// =============================================================================================

fn columns_of(st: &StudioUi, wf: &StudioWorkflow, id: u64, port: usize, kind: ColKind) -> Vec<String> {
    let ins = st.executor.inputs(wf, id);
    let Some(d) = ins.iter().find(|(p, _)| *p == port).and_then(|(_, v)| v.data()) else { return vec![] };
    let t = &d.table;
    t.columns
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            let ty = t.types.get(*i).copied().unwrap_or_default();
            match kind {
                ColKind::Numeric => ty.is_numeric(),
                ColKind::Categorical => !ty.is_numeric(),
                _ => true,
            }
        })
        .map(|(_, c)| c.clone())
        .collect()
}

fn as_str(v: Option<&J>) -> String {
    match v {
        Some(J::String(s)) => s.clone(),
        Some(J::Null) | None => String::new(),
        Some(v) => v.to_string(),
    }
}

fn inspector(app: &mut VizApp, st: &mut StudioUi, ui: &mut Ui, t: &Tokens) {
    let Some(wf) = app.project.studio.get(st.current).cloned() else { return };
    let Some(w) = st.selected.and_then(|s| wf.widget(s)).cloned() else {
        ui.label(RichText::new("Workflow").strong().size(13.0));
        let mut name = wf.name.clone();
        if ui.text_edit_singleline(&mut name).changed()
            && let Some(f) = app.project.studio.get_mut(st.current)
        {
            f.name = name;
        }
        if !wf.description.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new(&wf.description).size(11.5).color(t.muted));
        }
        ui.add_space(10.0);
        ui.label(RichText::new(format!("{} widgets · {} links", wf.widgets.len(), wf.links.len())).size(11.0).color(t.muted));
        ui.add_space(10.0);
        ui.label(RichText::new("Select a widget to edit its settings and see its window below.").size(11.5).color(t.muted));
        registry_list(app, st, ui, t);
        return;
    };
    let Some(ty) = studio::widget_type(&w.kind) else { return };
    let mut changes: Vec<(String, J)> = vec![];
    let mut label = w.label.clone();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 5.0, cat_color(ty.category));
            ui.label(RichText::new(ty.name).strong().size(13.0));
        });
        ui.label(RichText::new(ty.description).size(11.0).color(t.muted));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Label");
            ui.text_edit_singleline(&mut label);
        });
        ui.separator();
        let learners: Vec<String> = st.executor.inputs(&wf, w.id).iter().find_map(|(_, v)| v.eval().map(|e| e.learners.clone())).unwrap_or_default();
        let classes: Vec<String> = st.executor.inputs(&wf, w.id).iter().find_map(|(_, v)| v.eval().map(|e| e.classes.clone())).unwrap_or_default();
        for pd in &ty.params {
            let cur = w.settings.get(pd.key).cloned().unwrap_or_else(|| pd.default.clone());
            ui.label(RichText::new(pd.label).size(11.5)).on_hover_text(pd.help);
            let id = ui.scope_id().with(("st-param", w.id, pd.key));
            match &pd.kind {
                ParamKind::Table => {
                    let names: Vec<String> = app.built.tables.keys().cloned().collect();
                    let mut s = as_str(Some(&cur));
                    if combo(ui, id, &mut s, &names, false) {
                        changes.push((pd.key.into(), J::from(s)));
                    }
                }
                ParamKind::Column { kind, port, optional } => {
                    let opts = columns_of(st, &wf, w.id, *port, *kind);
                    let mut s = as_str(Some(&cur));
                    if combo(ui, id, &mut s, &opts, *optional) {
                        changes.push((pd.key.into(), J::from(s)));
                    }
                }
                ParamKind::Columns { kind, port } => {
                    let opts = columns_of(st, &wf, w.id, *port, *kind);
                    let mut set: Vec<String> = cur.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
                    let mut changed = false;
                    egui::CollapsingHeader::new(RichText::new(if set.is_empty() { "(all suitable)".to_string() } else { format!("{} selected", set.len()) }).size(11.0)).id_salt(id).show(ui, |ui| {
                        for c in &opts {
                            let mut on = set.contains(c);
                            if ui.checkbox(&mut on, c).changed() {
                                changed = true;
                                if on { set.push(c.clone()) } else { set.retain(|x| x != c) }
                            }
                        }
                        if opts.is_empty() {
                            ui.label(RichText::new("Connect data first").size(10.5).color(t.muted));
                        }
                    });
                    if changed {
                        changes.push((pd.key.into(), json!(set)));
                    }
                }
                ParamKind::Choice(opts) => {
                    let cur_s = as_str(Some(&cur));
                    let shown = opts.iter().find(|(k, _)| *k == cur_s).map(|(_, l)| *l).unwrap_or(&cur_s);
                    egui::ComboBox::from_id_salt(id).width(ui.available_width() - 8.0).selected_text(shown).show_ui(ui, |ui| {
                        for (k, l) in opts.iter() {
                            if ui.selectable_label(cur_s == *k, *l).clicked() {
                                changes.push((pd.key.into(), J::from(*k)));
                            }
                        }
                    });
                }
                ParamKind::Int(lo, hi) => {
                    let mut v = cur.as_i64().unwrap_or(*lo);
                    if ui.add(egui::DragValue::new(&mut v).range(*lo..=*hi)).changed() {
                        changes.push((pd.key.into(), J::from(v)));
                    }
                }
                ParamKind::Float(lo, hi) => {
                    let mut v = cur.as_f64().unwrap_or(*lo);
                    if ui.add(egui::DragValue::new(&mut v).range(*lo..=*hi).speed(((hi - lo) / 200.0).max(0.001))).changed() {
                        changes.push((pd.key.into(), J::from(v)));
                    }
                }
                ParamKind::Bool => {
                    let mut v = cur.as_bool().unwrap_or(false);
                    if ui.checkbox(&mut v, "").changed() {
                        changes.push((pd.key.into(), J::from(v)));
                    }
                }
                ParamKind::Text | ParamKind::File => {
                    let mut s = as_str(Some(&cur));
                    if ui.add(egui::TextEdit::singleline(&mut s).desired_width(f32::INFINITY)).changed() {
                        changes.push((pd.key.into(), J::from(s)));
                    }
                }
                ParamKind::Code => {
                    let mut s = as_str(Some(&cur));
                    if ui.add(egui::TextEdit::multiline(&mut s).code_editor().desired_rows(5).desired_width(f32::INFINITY)).changed() {
                        changes.push((pd.key.into(), J::from(s)));
                    }
                }
                ParamKind::Learner | ParamKind::Class => {
                    let opts = if matches!(pd.kind, ParamKind::Learner) { &learners } else { &classes };
                    let i = cur.as_u64().unwrap_or(0) as usize;
                    let shown = opts.get(i).cloned().unwrap_or_else(|| if opts.is_empty() { "(connect evaluation results)".into() } else { opts[0].clone() });
                    egui::ComboBox::from_id_salt(id).width(ui.available_width() - 8.0).selected_text(shown).show_ui(ui, |ui| {
                        for (k, l) in opts.iter().enumerate() {
                            if ui.selectable_label(k == i, l).clicked() {
                                changes.push((pd.key.into(), J::from(k)));
                            }
                        }
                    });
                }
                ParamKind::Registered => {
                    let mut names: Vec<String> = app.project.models.iter().map(|m| m.name.clone()).collect();
                    names.dedup();
                    let mut s = as_str(Some(&cur));
                    if combo(ui, id, &mut s, &names, false) {
                        changes.push((pd.key.into(), J::from(s)));
                    }
                }
            }
            ui.add_space(3.0);
        }
        ui.separator();
        actions(app, st, ui, t, &wf, &w);
    });
    if let Some(m) = app.project.studio.get_mut(st.current).and_then(|f| f.widget_mut(w.id)) {
        for (k, v) in changes {
            m.settings.insert(k, v);
        }
        if label != m.label {
            m.label = label;
        }
    }
}

fn combo(ui: &mut Ui, id: egui::Id, cur: &mut String, opts: &[String], optional: bool) -> bool {
    let mut changed = false;
    let shown = if cur.is_empty() { "(none)".to_string() } else { cur.clone() };
    egui::ComboBox::from_id_salt(id).width(ui.available_width() - 8.0).selected_text(shown).show_ui(ui, |ui| {
        if optional && ui.selectable_label(cur.is_empty(), "(none)").clicked() {
            cur.clear();
            changed = true;
        }
        for o in opts {
            if ui.selectable_label(cur == o, o).clicked() {
                *cur = o.clone();
                changed = true;
            }
        }
    });
    if opts.is_empty() {
        ui.label(RichText::new("Connect data to choose").size(10.0));
    }
    changed
}

/// Buttons for deploy widgets: register a model, save scored data as a table, export.
fn actions(app: &mut VizApp, st: &mut StudioUi, ui: &mut Ui, t: &Tokens, wf: &StudioWorkflow, w: &studio::StudioWidget) {
    let out = st.executor.get(w.id);
    match w.kind.as_str() {
        "deploy.register" => {
            let ins = st.executor.inputs(wf, w.id);
            let model = ins.iter().find_map(|(_, v)| v.model().cloned());
            let ev = ins.iter().find_map(|(_, v)| v.eval().cloned());
            let enabled = model.as_ref().is_some_and(|m| m.fitted.is_some() && m.train.is_some());
            if ui.add_enabled(enabled, egui::Button::new("Register new version")).clicked()
                && let Some(m) = model
                && let (Some(f), Some(train)) = (&m.fitted, &m.train)
            {
                let name = as_str(w.settings.get("name"));
                let name = if name.trim().is_empty() { m.spec.name.clone() } else { name };
                let metrics = exec::model_metrics(f, train, ev.as_deref());
                let mut rec = registry::register(&name, f, &train.table, metrics, &format!("{} / {}", wf.name, w.label), &as_str(w.settings.get("description")));
                rec.version = registry::next_version(&app.project.models, &name);
                let saved = vizcraft_mlops::Registry::open_default().save(&rec);
                st.message = Some(match saved {
                    Ok(r) => (format!("Registered {} v{}", r.name, rec.version), false),
                    Err(e) => (format!("Registered in project; disk registry failed: {e}"), true),
                });
                app.project.models.push(rec);
                app.dirty = true;
            }
        }
        "deploy.save" => {
            let data = st.executor.inputs(wf, w.id).iter().find_map(|(_, v)| v.data().cloned());
            if ui.add_enabled(data.is_some(), egui::Button::new("Save table to the model")).clicked()
                && let Some(d) = data
            {
                let name = as_str(w.settings.get("name"));
                let name = if name.trim().is_empty() { "Scored".to_string() } else { name };
                crate::studio::save_table(app, &name, &d.table);
                st.message = Some((format!("Saved table \"{name}\" ({} rows)", d.table.rows.len()), false));
            }
        }
        _ => {}
    }
    if let Some(o) = out.as_deref() {
        if !o.message.is_empty() && o.state != State::Ok {
            ui.label(RichText::new(&o.message).size(11.0).color(if o.state == State::Error { t.danger } else { t.muted }));
        }
        if o.ms > 0.0 {
            ui.label(RichText::new(format!("computed in {:.1} ms", o.ms)).size(10.0).color(t.muted));
        }
    }
    if w.kind.starts_with("deploy.") {
        registry_list(app, st, ui, t);
    }
}

/// Saves a table into the project as an entered data sheet so reports can use it.
pub fn save_table(app: &mut VizApp, name: &str, t: &TableData) {
    let mut sheet = vizcraft_core::doc::DataSheet { name: name.to_string(), ..Default::default() };
    sheet.cells.push(t.columns.iter().map(|c| c.clone()).collect());
    for r in &t.rows {
        sheet.cells.push(r.iter().map(|v| v.text()).collect());
    }
    app.project.sheets.retain(|s| s.name != name);
    app.project.sheets.push(sheet);
    app.refresh();
}

fn registry_list(app: &mut VizApp, st: &mut StudioUi, ui: &mut Ui, t: &Tokens) {
    ui.add_space(10.0);
    ui.label(RichText::new("Model registry").strong().size(12.5));
    if app.project.models.is_empty() {
        ui.label(RichText::new("No models yet. Add Deploy → Register Model after a trained model.").size(11.0).color(t.muted));
        return;
    }
    let mut stage_change = None;
    let mut export = None;
    for (i, m) in app.project.models.iter().enumerate() {
        egui::Frame::new().fill(t.panel_alt).corner_radius(6.0).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(format!("{} v{}", m.name, m.version)).strong().size(11.5));
            ui.label(RichText::new(format!("{} · {} · {}", m.algorithm, m.task, m.created)).size(10.0).color(t.muted));
            let top: Vec<String> = m.metrics.iter().take(3).map(|(k, v)| format!("{k} {v:.3}")).collect();
            if !top.is_empty() {
                ui.label(RichText::new(top.join(" · ")).size(10.0));
            }
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt(("st-stage", i)).width(96.0).selected_text(m.stage.label()).show_ui(ui, |ui| {
                    for s in registry::Stage::ALL {
                        if ui.selectable_label(m.stage == s, s.label()).clicked() {
                            stage_change = Some((i, s));
                        }
                    }
                });
                if ui.small_button("Export").on_hover_text("Write the model as JSON and a standalone Python scorer").clicked() {
                    export = Some(i);
                }
            });
        });
        ui.add_space(3.0);
    }
    if let Some((i, s)) = stage_change {
        let name = app.project.models[i].name.clone();
        if s == registry::Stage::Production {
            for m in app.project.models.iter_mut().filter(|m| m.name == name && m.stage == registry::Stage::Production) {
                m.stage = registry::Stage::Archived;
            }
        }
        app.project.models[i].stage = s;
        let m = &app.project.models[i];
        let _ = vizcraft_mlops::Registry::open_default().set_stage(&m.name, m.version, s);
        app.dirty = true;
    }
    if let Some(i) = export {
        let m = app.project.models[i].clone();
        let dir = vizcraft_mlops::default_dir().join("exports");
        let stem = format!("{}_v{}", vizcraft_mlops::slug(&m.name), m.version);
        let r = std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::write(dir.join(format!("{stem}.json")), registry::to_json(&m)))
            .and_then(|_| std::fs::write(dir.join(format!("{stem}.py")), registry::python_script(&m)));
        st.message = Some(match r {
            Ok(()) => (format!("Exported to {}", dir.display()), false),
            Err(e) => (e.to_string(), true),
        });
    }
}

// =============================================================================================
// Widget window
// =============================================================================================

fn window(app: &mut VizApp, st: &mut StudioUi, ui: &mut Ui, t: &Tokens) {
    let Some(wf) = app.project.studio.get(st.current).cloned() else { return };
    let Some(w) = st.selected.and_then(|s| wf.widget(s)).cloned() else {
        ui.centered_and_justified(|ui| ui.label(RichText::new("Select a widget to open its window").color(t.muted)));
        return;
    };
    let out = st.executor.get(w.id);
    let ins = st.executor.inputs(&wf, w.id);
    ui.horizontal(|ui| {
        ui.label(RichText::new(&w.label).strong().size(13.0));
        if let Some(o) = out.as_deref() {
            ui.label(RichText::new(&o.summary).size(11.0).color(t.muted));
        }
    });
    ui.separator();
    let Some(o) = out else { return };
    if o.state == State::Error || (o.state == State::Idle && !o.message.is_empty()) {
        ui.label(RichText::new(&o.message).color(if o.state == State::Error { t.danger } else { t.muted }));
        return;
    }
    let in_data = ins.iter().find_map(|(_, v)| v.data().cloned());
    let in_eval = ins.iter().find_map(|(_, v)| v.eval().cloned());
    let out_data = o.outputs.iter().flatten().find_map(|v| v.data().cloned());
    let s = |k: &str| as_str(w.settings.get(k));
    let n = |k: &str| w.settings.get(k).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    egui::ScrollArea::both().id_salt(("st-win", w.id)).auto_shrink([false, false]).show(ui, |ui| {
        // Widget-specific views computed here from inputs.
        match w.kind.as_str() {
            "eval.confusion" => {
                if let Some(r) = in_eval {
                    plots::confusion(ui, t, &r, n("learner"), &s("show"));
                }
                return;
            }
            "eval.roc" | "eval.lift" | "eval.calibration" => {
                if let Some(r) = in_eval {
                    plots::curves(ui, t, &r, &w.kind, n("class"), &s("curve"), w.settings.get("bins").and_then(|v| v.as_u64()).unwrap_or(10) as usize);
                }
                return;
            }
            "viz.scatter" | "viz.distributions" | "viz.box" | "viz.heatmap" | "viz.correlations" | "viz.sieve" | "viz.mosaic" | "viz.line" => {
                if let Some(d) = &in_data {
                    plots::viz(ui, t, &w.kind, &d.table, &w.settings);
                }
                return;
            }
            _ => {}
        }
        match &o.view {
            View::None => {
                if let Some(d) = out_data.or(in_data) {
                    plots::data_table(ui, t, &d.table, d.target.as_deref());
                } else if let Some(r) = in_eval {
                    plots::scores(ui, t, &eval::scores(&r), r.is_classification());
                } else {
                    ui.label(RichText::new("Nothing to show yet.").color(t.muted));
                }
            }
            v => plots::view(ui, t, v),
        }
    });
}
