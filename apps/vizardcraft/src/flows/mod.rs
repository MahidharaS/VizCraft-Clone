//! Flows: Fabric/KNIME/Orange-style node pipelines (canvas, palette, inspector, previews),
//! notebooks (Markdown / Python / DAX / M cells) and machine learning. Output nodes become model
//! tables (`TableSource::Flow`), like a Fabric lakehouse.

mod canvas;
mod glyphs;
mod inspector;
mod notebook;
pub mod preview;

use std::collections::{HashMap, HashSet};

use egui::{Color32, RichText, Ui, vec2};
use vizardcraft_core::doc::{Flow, FlowNode, ModelTable, Notebook, NotebookCell, TableSource};
use vizardcraft_core::flow::{self, Ctx, Executor, Status};

use crate::app::VizApp;
use crate::theme::Tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Flow(usize),
    Notebook(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub pan: egui::Vec2,
    pub zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        View { pan: vec2(30.0, 30.0), zoom: 1.0 }
    }
}

pub struct FlowsUi {
    pub tab: Tab,
    pub executor: Executor,
    pub selected: Option<u64>,
    pub views: HashMap<String, View>,
    pub palette_search: String,
    pub auto_run: bool,
    pub preview_port: usize,
    pub renaming: Option<(Tab, String)>,
    pub(crate) connecting: Option<canvas::Connect>,
    pub(crate) dragging_node: Option<u64>,
    pub(crate) palette_drag: Option<String>,
    pub(crate) last_edit: f64,
    pub(crate) cp_guard: Option<(String, f64)>,
    pub(crate) fit_request: bool,
    pub(crate) env_checked: bool,
    pub(crate) nb: notebook::NotebookState,
    pub(crate) run_request: Option<RunRequest>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunRequest {
    All,
    Node(u64),
}

impl Default for FlowsUi {
    fn default() -> Self {
        FlowsUi {
            tab: Tab::Flow(0),
            executor: Executor::default(),
            selected: None,
            views: HashMap::new(),
            palette_search: String::new(),
            auto_run: true,
            preview_port: 0,
            renaming: None,
            connecting: None,
            dragging_node: None,
            palette_drag: None,
            last_edit: 0.0,
            cp_guard: None,
            fit_request: true,
            env_checked: false,
            nb: Default::default(),
            run_request: None,
        }
    }
}

pub fn category_color(cat: &str) -> Color32 {
    match cat {
        "Data" => Color32::from_rgb(0x3B, 0x82, 0xF6),
        "Transform" => Color32::from_rgb(0x0E, 0xA5, 0x99),
        "Unsupervised" => Color32::from_rgb(0x8B, 0x5C, 0xF6),
        "Supervised" => Color32::from_rgb(0xEA, 0x8C, 0x0C),
        "Statistics" => Color32::from_rgb(0xDB, 0x27, 0x77),
        "Scripting" => Color32::from_rgb(0x16, 0xA3, 0x4A),
        _ => Color32::from_rgb(0x47, 0x55, 0x69),
    }
}

// =============================================================================================
// Entry points
// =============================================================================================

pub fn show(app: &mut VizApp, ui: &mut Ui) {
    let mut st = std::mem::take(&mut app.flows_ui);
    let t = app.tokens();
    normalize_tab(app, &mut st);
    tab_strip(app, &mut st, ui, &t);
    match st.tab {
        Tab::Flow(i) if i < app.project.flows.len() => flow_tab(app, &mut st, ui, &t, i),
        Tab::Notebook(i) if i < app.project.notebooks.len() => notebook::show(app, &mut st, ui, &t, i),
        _ => empty_state(app, &mut st, ui, &t),
    }
    app.flows_ui = st;
}

pub fn dialogs(app: &mut VizApp, _ctx: &egui::Context) {
    let mut st = std::mem::take(&mut app.flows_ui);
    if !st.env_checked {
        st.env_checked = true;
        if let Ok(v) = std::env::var("VIZARDCRAFT_OPEN") {
            match v.as_str() {
                "flow-example" | "flow-kmeans" | "flow-regression" | "flow-join" => {
                    let i = if v == "flow-regression" {
                        add_example(app, &mut st, "regression")
                    } else {
                        app.project.flows.iter().position(|f| f.name == "Customer segments").unwrap_or_else(|| add_example(app, &mut st, "segments"))
                    };
                    st.tab = Tab::Flow(i);
                    let want = match v.as_str() {
                        "flow-regression" => "ml.score",
                        "flow-join" => "tf.join",
                        _ => "ml.kmeans",
                    };
                    st.selected = app.project.flows.get(i).and_then(|f| f.nodes.iter().find(|n| n.kind == want).map(|n| n.id));
                    st.fit_request = true;
                }
                "notebook" => {
                    new_notebook(app, &mut st);
                    st.nb.run_all = true;
                }
                _ => {}
            }
        }
    }
    app.flows_ui = st;
}

pub fn new_flow(app: &mut VizApp) {
    let mut st = std::mem::take(&mut app.flows_ui);
    create_flow(app, &mut st);
    app.flows_ui = st;
}

/// Opens (or creates) a notebook in the flows view (for the ribbon's Notebook button).
#[allow(dead_code)]
pub fn open_notebook(app: &mut VizApp) {
    let mut st = std::mem::take(&mut app.flows_ui);
    if app.project.notebooks.is_empty() {
        new_notebook(app, &mut st);
    } else {
        st.tab = Tab::Notebook(0);
    }
    app.view = crate::app::View::Flows;
    app.pq_open = false;
    app.flows_ui = st;
}

fn create_flow(app: &mut VizApp, st: &mut FlowsUi) {
    let name = flow::unique_flow_name(&app.project, "Flow 1");
    let name = if name == "Flow 1" { name } else { flow::unique_flow_name(&app.project, "Flow") };
    app.checkpoint("New flow");
    // Start from the first data sheet or query.
    let mut nodes = Vec::new();
    if let Some(s) = app.project.sheets.first().map(|s| s.name.clone()) {
        let id = app.project.next_id();
        nodes.push(FlowNode { id, kind: "source.sheet".into(), label: s.clone(), x: 60.0, y: 120.0, config: [("name".to_string(), serde_json::json!(s))].into_iter().collect() });
    } else if let Some(q) = app.project.queries.queries.first().map(|q| q.name.clone()) {
        let id = app.project.next_id();
        nodes.push(FlowNode { id, kind: "source.query".into(), label: q.clone(), x: 60.0, y: 120.0, config: [("name".to_string(), serde_json::json!(q))].into_iter().collect() });
    }
    st.selected = nodes.first().map(|n| n.id);
    app.project.flows.push(Flow { name: name.clone(), nodes, links: vec![], description: String::new() });
    st.tab = Tab::Flow(app.project.flows.len() - 1);
    st.fit_request = false;
    st.views.insert(name.clone(), View::default());
    app.view = crate::app::View::Flows;
    app.pq_open = false;
    app.toast(format!("New flow “{name}”: drag nodes from the palette and connect their ports."));
}

fn new_notebook(app: &mut VizApp, st: &mut FlowsUi) {
    let base = "Notebook";
    let name = (1..1000).map(|i| format!("{base} {i}")).find(|n| !app.project.notebooks.iter().any(|x| &x.name == n)).unwrap_or_else(|| base.to_string());
    app.checkpoint("New notebook");
    // The biggest loaded table (usually the fact table).
    let first_table = app.built.tables.iter().max_by_key(|(_, t)| t.rows.len()).map(|(k, _)| k.clone()).unwrap_or_else(|| "Sales".into());
    let py = flow::py_ident(&first_table);
    let mut cells = vec![
        ("markdown", format!("# {name}\nEvery model table is a pandas DataFrame named after it (e.g. `{py}`); `tables` maps names to frames.\nRun a cell with **Ctrl+Enter**. DAX cells run `EVALUATE` queries against the model; M cells evaluate Power Query.")),
        ("python", format!("{py}.describe()")),
    ];
    if app.project.model.measures.iter().any(|m| m.name == "Total Sales") && app.project.model.tables.iter().any(|t| t.name == "Store") {
        cells.push(("dax", "EVALUATE\nSUMMARIZECOLUMNS(Store[Region], \"Sales\", [Total Sales], \"Orders\", [Orders])\nORDER BY [Sales] DESC".into()));
        cells.push(("python", format!("by_channel = {py}.groupby('Channel')['Quantity'].sum()\nprint('Units by channel:')\nprint(by_channel)\nby_channel.sort_values(ascending=False)")));
    } else {
        cells.push(("dax", format!("EVALUATE TOPN(10, '{first_table}')")));
    }
    let cells = cells.into_iter().map(|(k, s)| NotebookCell { id: app.project.next_id(), kind: k.into(), source: s }).collect();
    app.project.notebooks.push(Notebook { name, cells });
    st.tab = Tab::Notebook(app.project.notebooks.len() - 1);
    app.view = crate::app::View::Flows;
    app.pq_open = false;
}

fn add_example(app: &mut VizApp, st: &mut FlowsUi, which: &str) -> usize {
    app.checkpoint("Example flow");
    let f = if which == "regression" { flow::example_regression(&mut app.project) } else { flow::example_segments(&mut app.project) };
    let name = f.name.clone();
    app.project.flows.push(f);
    let i = app.project.flows.len() - 1;
    st.tab = Tab::Flow(i);
    st.selected = None;
    st.fit_request = true;
    app.toast(format!("Added the example flow “{name}”."));
    i
}

fn normalize_tab(app: &VizApp, st: &mut FlowsUi) {
    match st.tab {
        Tab::Flow(i) if i >= app.project.flows.len() => {
            st.tab = if !app.project.flows.is_empty() { Tab::Flow(app.project.flows.len() - 1) } else if !app.project.notebooks.is_empty() { Tab::Notebook(0) } else { Tab::Flow(0) };
        }
        Tab::Notebook(i) if i >= app.project.notebooks.len() => {
            st.tab = if !app.project.notebooks.is_empty() { Tab::Notebook(app.project.notebooks.len() - 1) } else { Tab::Flow(0) };
        }
        _ => {}
    }
}

// =============================================================================================
// Settings edits (undo grouping)
// =============================================================================================

/// Records an undo step for a settings edit, grouping quick successive edits of one field.
pub(crate) fn checkpoint_grouped(app: &mut VizApp, st: &mut FlowsUi, key: &str, label: &str) {
    let now = crate::app::now();
    let same = st.cp_guard.as_ref().is_some_and(|(k, t)| k == key && now - t < 2.0);
    if !same {
        app.checkpoint(label);
    }
    st.cp_guard = Some((key.to_string(), now));
    st.last_edit = now;
}

pub(crate) fn host_ctx<'a>(app: &'a VizApp, host: &'a vizardcraft_connectors::DesktopHost) -> Ctx<'a> {
    Ctx { project: &app.project, host, tables: Some(&app.built.tables) }
}

/// Adds a flow output to the model as a table.
pub(crate) fn add_to_model(app: &mut VizApp, flow_i: usize, node: u64) {
    let Some(f) = app.project.flows.get(flow_i).cloned() else { return };
    let Some(n) = f.nodes.iter().find(|n| n.id == node) else { return };
    let spec = flow::spec(&f.name, node);
    let wanted = match flow::setting(n, "name") {
        serde_json::Value::String(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => n.label.clone(),
    };
    if let Some(t) = app.project.model.tables.iter().find(|t| t.source == TableSource::Flow(spec.clone())) {
        let name = t.name.clone();
        app.mark_data_dirty();
        app.toast(format!("“{name}” is already a model table: refreshing it."));
        return;
    }
    let name = app.project.free_table_name(&wanted);
    app.checkpoint("Add flow output to model");
    app.project.model.tables.push(ModelTable { name: name.clone(), source: TableSource::Flow(spec), storage_mode: "Import".into(), description: format!("Output of flow “{}”.", f.name), ..ModelTable::default() });
    if let Some(n) = app.project.flows.get_mut(flow_i).and_then(|f| f.nodes.iter_mut().find(|n| n.id == node))
        && n.kind == "out.table"
    {
        n.config.insert("name".into(), serde_json::json!(name.clone()));
    }
    app.mark_data_dirty();
    app.toast(format!("“{name}” is now a model table (refreshing)."));
}

pub(crate) fn in_model(app: &VizApp, flow_i: usize, node: u64) -> Option<String> {
    let f = app.project.flows.get(flow_i)?;
    let spec = flow::spec(&f.name, node);
    app.project.model.tables.iter().find(|t| t.source == TableSource::Flow(spec.clone())).map(|t| t.name.clone())
}

fn rename_flow(app: &mut VizApp, i: usize, new: &str) {
    let Some(old) = app.project.flows.get(i).map(|f| f.name.clone()) else { return };
    if new.trim().is_empty() || new == old {
        return;
    }
    if app.project.flows.iter().any(|f| f.name.eq_ignore_ascii_case(new)) {
        app.error(format!("A flow named “{new}” already exists."));
        return;
    }
    app.checkpoint("Rename flow");
    if let Some(f) = app.project.flows.get_mut(i) {
        f.name = new.to_string();
    }
    // Keep model tables pointing at this flow.
    for t in app.project.model.tables.iter_mut() {
        if let TableSource::Flow(spec) = &t.source {
            let (fname, node) = flow::parse_spec(spec);
            if fname == old {
                t.source = TableSource::Flow(format!("{new}::{node}"));
            }
        }
    }
}

// =============================================================================================
// Chrome
// =============================================================================================

fn empty_state(app: &mut VizApp, st: &mut FlowsUi, ui: &mut Ui, t: &Tokens) {
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.2);
            let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), egui::Sense::hover());
            crate::icons::paint(ui.painter(), r, "flow", t.accent);
            ui.add_space(8.0);
            ui.label(RichText::new("Pipelines, notebooks and machine learning").size(18.0).strong().color(t.text));
            ui.label(RichText::new("Chain steps on a canvas (prepare, cluster, predict, score) and save the results as model tables.").color(t.muted));
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 640.0).max(0.0) / 2.0);
                if ui.add(egui::Button::new(RichText::new("New flow").color(Color32::WHITE)).fill(t.accent)).clicked() {
                    create_flow(app, st);
                }
                if ui.button("New notebook").clicked() {
                    new_notebook(app, st);
                }
                if ui.button("Example: Customer segments (k-Means)").clicked() {
                    add_example(app, st, "segments");
                }
                if ui.button("Example: Predict line sales (forest)").clicked() {
                    add_example(app, st, "regression");
                }
            });
        });
    });
}

fn tab_strip(app: &mut VizApp, st: &mut FlowsUi, ui: &mut Ui, t: &Tokens) {
    egui::Panel::top("flows_tabs").exact_size(36.0).frame(egui::Frame::new().fill(t.panel_alt).inner_margin(egui::Margin { left: 8, right: 8, top: 6, bottom: 0 }).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let mut tabs: Vec<(Tab, String, &str)> = app.project.flows.iter().enumerate().map(|(i, f)| (Tab::Flow(i), f.name.clone(), "flow")).collect();
            tabs.extend(app.project.notebooks.iter().enumerate().map(|(i, n)| (Tab::Notebook(i), n.name.clone(), "python")));
            let mut action: Option<(&str, Tab)> = None;
            for (tab, name, icon) in tabs {
                if let Some((rt, text)) = st.renaming.as_mut()
                    && *rt == tab
                {
                    let r = ui.add(egui::TextEdit::singleline(text).desired_width(140.0));
                    r.request_focus();
                    if r.lost_focus() {
                        action = Some(("rename_done", tab));
                    }
                    continue;
                }
                let active = st.tab == tab;
                let w = crate::widgets::text_w(ui, &name) + 40.0;
                let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), egui::Sense::click());
                let fill = if active { t.panel } else if resp.hovered() { t.accent_soft } else { t.panel_alt };
                ui.painter().rect(rect, egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, fill, egui::Stroke::new(1.0, if active { t.border } else { Color32::TRANSPARENT }), egui::StrokeKind::Inside);
                if active {
                    ui.painter().line_segment([rect.left_top() + vec2(6.0, 1.0), rect.right_top() + vec2(-6.0, 1.0)], egui::Stroke::new(2.5, t.accent));
                    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], egui::Stroke::new(2.0, t.panel));
                }
                crate::icons::paint(ui.painter(), egui::Rect::from_center_size(rect.left_center() + vec2(15.0, 0.0), vec2(15.0, 15.0)), icon, if active { t.accent } else { t.muted });
                ui.painter().text(rect.left_center() + vec2(28.0, 0.0), egui::Align2::LEFT_CENTER, &name, egui::FontId::proportional(12.5), if active { t.text } else { t.muted });
                let resp = resp.on_hover_text(if matches!(tab, Tab::Flow(_)) { "Flow: double-click to rename, right-click for more" } else { "Notebook" });
                if resp.clicked() {
                    action = Some(("select", tab));
                }
                if resp.double_clicked() {
                    action = Some(("rename", tab));
                }
                resp.context_menu(|ui| {
                    if ui.button("Rename").clicked() {
                        action = Some(("rename", tab));
                        ui.close();
                    }
                    if ui.button("Duplicate").clicked() {
                        action = Some(("duplicate", tab));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(RichText::new("Delete").color(t.danger)).clicked() {
                        action = Some(("delete", tab));
                        ui.close();
                    }
                });
            }
            ui.add_space(4.0);
            let plus = ui.add(egui::Button::new(RichText::new("+").size(15.0)).frame(false).min_size(vec2(28.0, 26.0))).on_hover_text("New flow, notebook or example");
            egui::Popup::menu(&plus).show(|ui| {
                if ui.button("New flow").clicked() {
                    create_flow(app, st);
                    ui.close();
                }
                if ui.button("New notebook").clicked() {
                    new_notebook(app, st);
                    ui.close();
                }
                ui.separator();
                ui.label(RichText::new("Examples").size(11.0).color(t.muted));
                if ui.button("Customer segments (k-Means)").clicked() {
                    add_example(app, st, "segments");
                    ui.close();
                }
                if ui.button("Predict line sales (random forest vs linear)").clicked() {
                    add_example(app, st, "regression");
                    ui.close();
                }
            });
            match action {
                Some(("select", tab)) => {
                    if st.tab != tab {
                        st.tab = tab;
                        st.selected = None;
                        st.preview_port = 0;
                        st.fit_request = !st.views.contains_key(&tab_name(app, tab));
                    }
                }
                Some(("rename", tab)) => st.renaming = Some((tab, tab_name(app, tab))),
                Some(("rename_done", tab)) => {
                    if let Some((_, new)) = st.renaming.take() {
                        let new = new.trim().to_string();
                        match tab {
                            Tab::Flow(i) => {
                                let old = tab_name(app, tab);
                                rename_flow(app, i, &new);
                                if let Some(v) = st.views.remove(&old) {
                                    st.views.insert(new, v);
                                }
                                st.executor.clear();
                            }
                            Tab::Notebook(i) => {
                                if !new.is_empty() {
                                    app.checkpoint("Rename notebook");
                                    if let Some(n) = app.project.notebooks.get_mut(i) {
                                        n.name = new;
                                    }
                                }
                            }
                        }
                    }
                }
                Some(("duplicate", tab)) => match tab {
                    Tab::Flow(i) => {
                        if let Some(mut f) = app.project.flows.get(i).cloned() {
                            f.name = flow::unique_flow_name(&app.project, &format!("{} copy", f.name));
                            app.checkpoint("Duplicate flow");
                            app.project.flows.push(f);
                            st.tab = Tab::Flow(app.project.flows.len() - 1);
                        }
                    }
                    Tab::Notebook(i) => {
                        if let Some(mut n) = app.project.notebooks.get(i).cloned() {
                            n.name = format!("{} copy", n.name);
                            for c in n.cells.iter_mut() {
                                c.id = app.project.next_id();
                            }
                            app.checkpoint("Duplicate notebook");
                            app.project.notebooks.push(n);
                            st.tab = Tab::Notebook(app.project.notebooks.len() - 1);
                        }
                    }
                },
                Some(("delete", tab)) => {
                    let name = tab_name(app, tab);
                    app.checkpoint("Delete");
                    match tab {
                        Tab::Flow(i) => {
                            if i < app.project.flows.len() {
                                app.project.flows.remove(i);
                            }
                        }
                        Tab::Notebook(i) => {
                            if i < app.project.notebooks.len() {
                                app.project.notebooks.remove(i);
                            }
                        }
                    }
                    st.selected = None;
                    app.toast(format!("Deleted “{name}” (Ctrl+Z to undo)."));
                }
                _ => {}
            }
        });
    });
}

fn tab_name(app: &VizApp, tab: Tab) -> String {
    match tab {
        Tab::Flow(i) => app.project.flows.get(i).map(|f| f.name.clone()).unwrap_or_default(),
        Tab::Notebook(i) => app.project.notebooks.get(i).map(|n| n.name.clone()).unwrap_or_default(),
    }
}

fn flow_tab(app: &mut VizApp, st: &mut FlowsUi, ui: &mut Ui, t: &Tokens, fi: usize) {
    let host = vizardcraft_connectors::DesktopHost::default();
    // Run requests and auto-run (debounced while typing; Python waits for an explicit run).
    if let Some(req) = st.run_request.take()
        && let Some(f) = app.project.flows.get(fi)
    {
        let ctx = host_ctx(app, &host);
        let t0 = std::time::Instant::now();
        match req {
            RunRequest::All => {
                let errors = st.executor.run_all(&ctx, f);
                let ms = t0.elapsed().as_secs_f64() * 1000.0;
                if errors == 0 {
                    app.toast(format!("Ran {} nodes in {ms:.0} ms", f.nodes.len()));
                } else {
                    app.error(format!("{errors} node{} failed: select one to see why.", if errors == 1 { "" } else { "s" }));
                }
            }
            RunRequest::Node(id) => {
                if let Err(e) = st.executor.run(&ctx, f, id) {
                    app.error(e);
                }
            }
        }
    }
    let statuses = app.project.flows.get(fi).map(|f| st.executor.statuses(&host_ctx(app, &host), f)).unwrap_or_default();
    if st.auto_run && crate::app::now() - st.last_edit > 0.6 && statuses.values().any(|s| *s == Status::Stale) && st.dragging_node.is_none() {
        if let Some(f) = app.project.flows.get(fi) {
            let python = python_downstream(f);
            let ctx = host_ctx(app, &host);
            for n in &f.nodes {
                if !python.contains(&n.id) && statuses.get(&n.id) == Some(&Status::Stale) {
                    let _ = st.executor.run(&ctx, f, n.id);
                }
            }
        }
    } else if st.auto_run && statuses.values().any(|s| *s == Status::Stale) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
    }
    let statuses = app.project.flows.get(fi).map(|f| st.executor.statuses(&host_ctx(app, &host), f)).unwrap_or_default();

    toolbar(app, st, ui, t, fi, &statuses);
    palette(app, st, ui, t, fi);
    inspector::show(app, st, ui, t, fi, &statuses);
    preview_panel(app, st, ui, t, fi, &statuses);
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.canvas)).show(ui, |ui| {
        canvas::show(app, st, ui, t, fi, &statuses);
    });
}

/// Python script nodes and everything downstream of them (not auto-run).
fn python_downstream(f: &Flow) -> HashSet<u64> {
    let mut set: HashSet<u64> = f.nodes.iter().filter(|n| n.kind == "script.python").map(|n| n.id).collect();
    loop {
        let before = set.len();
        for l in &f.links {
            if set.contains(&l.0) {
                set.insert(l.2);
            }
        }
        if set.len() == before {
            break;
        }
    }
    set
}

/// A small × button (painted: no font glyph needed).
pub(crate) fn x_button(ui: &mut Ui, t: &Tokens, tip: &str) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 4.0, t.danger.gamma_multiply(0.12));
    }
    let c = if resp.hovered() { t.danger } else { t.muted };
    let m = r.center();
    ui.painter().line_segment([m - vec2(4.0, 4.0), m + vec2(4.0, 4.0)], egui::Stroke::new(1.5, c));
    ui.painter().line_segment([m - vec2(4.0, -4.0), m + vec2(4.0, -4.0)], egui::Stroke::new(1.5, c));
    resp.on_hover_text(tip)
}

fn tbtn(ui: &mut Ui, t: &Tokens, icon: &str, label: &str, tip: &str, primary: bool) -> egui::Response {
    let w = crate::widgets::text_w(ui, label) + if icon.is_empty() { 18.0 } else { 34.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 26.0), egui::Sense::click());
    let (fill, fg) = if primary { (if resp.hovered() { t.accent.gamma_multiply(0.85) } else { t.accent }, Color32::WHITE) } else { (if resp.hovered() { t.accent_soft } else { Color32::TRANSPARENT }, t.text) };
    ui.painter().rect_filled(rect, 5.0, fill);
    let mut x = rect.left() + 9.0;
    if icon == "play" {
        let c = egui::pos2(x + 5.0, rect.center().y);
        ui.painter().add(egui::Shape::convex_polygon(vec![c + vec2(-4.0, -5.5), c + vec2(5.5, 0.0), c + vec2(-4.0, 5.5)], fg, egui::Stroke::NONE));
        x += 17.0;
    } else if !icon.is_empty() {
        crate::icons::paint(ui.painter(), egui::Rect::from_min_size(egui::pos2(x - 1.0, rect.center().y - 8.0), vec2(16.0, 16.0)), icon, if primary { fg } else { t.accent });
        x += 18.0;
    }
    ui.painter().text(egui::pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(12.0), fg);
    resp.on_hover_text(tip)
}

fn toolbar(app: &mut VizApp, st: &mut FlowsUi, ui: &mut Ui, t: &Tokens, fi: usize, statuses: &HashMap<u64, Status>) {
    egui::Panel::top("flows_toolbar").exact_size(38.0).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::symmetric(8, 5)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if tbtn(ui, t, "play", "Run all", "Run every node of the flow", true).clicked() {
                st.run_request = Some(RunRequest::All);
            }
            if tbtn(ui, t, "play", "Run selected", "Run the selected node (and what it needs)", false).clicked()
                && let Some(id) = st.selected
            {
                st.run_request = Some(RunRequest::Node(id));
            }
            let mut auto = st.auto_run;
            ui.add(egui::Checkbox::new(&mut auto, RichText::new("Auto-run").size(12.0))).on_hover_text("Re-run changed nodes automatically (Python scripts run only on demand)");
            st.auto_run = auto;
            ui.add_space(8.0);
            let sel_kind = st.selected.and_then(|id| app.project.flows.get(fi).and_then(|f| f.nodes.iter().find(|n| n.id == id)).map(|n| n.kind.clone()));
            if let Some(id) = st.selected {
                let has_table = sel_kind.as_deref().and_then(flow::node_type).is_some_and(|nt| nt.outputs.first().is_some_and(|p| p.kind == flow::PortKind::Table));
                if has_table {
                    match in_model(app, fi, id) {
                        Some(name) => {
                            ui.label(RichText::new(format!("In model as “{name}”")).size(12.0).color(t.good));
                        }
                        None => {
                            if tbtn(ui, t, "model", "Add to model", "Make this node's output a table of the semantic model", false).clicked() {
                                add_to_model(app, fi, id);
                            }
                        }
                    }
                }
                if tbtn(ui, t, "", "Delete node", "Delete the selected node (Del)", false).clicked() {
                    canvas::delete_node(app, st, fi, id);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let errors = statuses.values().filter(|s| **s == Status::Error).count();
                let stale = statuses.values().filter(|s| **s == Status::Stale).count();
                let ok = statuses.values().filter(|s| **s == Status::Ok).count();
                let fname = app.project.flows.get(fi).map(|f| f.name.clone()).unwrap_or_default();
                let view = st.views.entry(fname).or_default();
                if ui.button("Fit").on_hover_text("Fit the flow in view").clicked() {
                    st.fit_request = true;
                }
                if ui.button("+").on_hover_text("Zoom in").clicked() {
                    view.zoom = (view.zoom * 1.2).min(2.5);
                }
                ui.label(RichText::new(format!("{:.0}%", view.zoom * 100.0)).size(11.5).color(t.muted));
                if ui.button("−").on_hover_text("Zoom out").clicked() {
                    view.zoom = (view.zoom / 1.2).max(0.3);
                }
                ui.add_space(12.0);
                let dot = |ui: &mut Ui, c: Color32, n: usize, label: &str| {
                    let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().circle_filled(r.center(), 4.0, c);
                    ui.label(RichText::new(format!("{n} {label}")).size(11.5).color(t.muted));
                };
                if errors > 0 {
                    dot(ui, t.danger, errors, "failed");
                }
                if stale > 0 {
                    dot(ui, Color32::from_rgb(0xA0, 0xA8, 0xB8), stale, "not run");
                }
                dot(ui, t.good, ok, "ok");
            });
        });
    });
}

fn palette(app: &mut VizApp, st: &mut FlowsUi, ui: &mut Ui, t: &Tokens, fi: usize) {
    egui::Panel::left("flows_palette").exact_size(212.0).resizable(false).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::symmetric(10, 10)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        ui.label(RichText::new("Nodes").strong().size(13.0).color(t.text));
        ui.add_space(4.0);
        ui.add(egui::TextEdit::singleline(&mut st.palette_search).hint_text("Search nodes").desired_width(f32::INFINITY));
        ui.add_space(6.0);
        let q = st.palette_search.to_lowercase();
        let cat = flow::catalog();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for c in flow::CATEGORIES {
                let items: Vec<&flow::NodeType> = cat.iter().filter(|n| n.category == *c && (q.is_empty() || n.label.to_lowercase().contains(&q) || n.help.to_lowercase().contains(&q))).collect();
                if items.is_empty() {
                    continue;
                }
                let col = category_color(c);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().circle_filled(r.center(), 3.5, col);
                    ui.label(RichText::new(c.to_uppercase()).size(10.5).strong().color(t.muted));
                });
                for nt in items {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), egui::Sense::click_and_drag());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, 5.0, t.accent_soft);
                    }
                    let ic = egui::Rect::from_center_size(rect.left_center() + vec2(13.0, 0.0), vec2(20.0, 20.0));
                    ui.painter().rect_filled(ic, 5.0, col.gamma_multiply(0.16));
                    glyphs::paint(ui.painter(), ic.shrink(3.0), nt.icon, col);
                    ui.painter().text(rect.left_center() + vec2(30.0, 0.0), egui::Align2::LEFT_CENTER, nt.label, egui::FontId::proportional(12.5), t.text);
                    let resp = resp.on_hover_text(format!("{}\nDrag onto the canvas, or click to add after the selected node.", nt.help));
                    if resp.drag_started() {
                        st.palette_drag = Some(nt.id.to_string());
                    }
                    if resp.clicked() {
                        canvas::add_node_after_selection(app, st, fi, nt.id);
                    }
                }
            }
        });
    });
}

fn preview_panel(app: &mut VizApp, st: &mut FlowsUi, ui: &mut Ui, t: &Tokens, fi: usize, statuses: &HashMap<u64, Status>) {
    let Some(f) = app.project.flows.get(fi) else { return };
    let Some(node) = st.selected.and_then(|id| f.nodes.iter().find(|n| n.id == id)) else { return };
    let nt = flow::node_type(&node.kind);
    egui::Panel::bottom("flows_preview").resizable(true).default_size(270.0).min_size(120.0).max_size(700.0).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::symmetric(12, 8)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        let status = statuses.get(&node.id).copied().unwrap_or(Status::Stale);
        let cached = st.executor.cached(&f.name, node.id).cloned();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Preview · {}", node.label)).strong().size(13.0).color(t.text));
            if let Some(nt) = &nt {
                let outs: Vec<(usize, &flow::Port)> = nt.outputs.iter().enumerate().filter(|(_, p)| p.kind == flow::PortKind::Table).collect();
                if outs.len() > 1 {
                    ui.add_space(8.0);
                    for (i, p) in outs {
                        if ui.selectable_label(st.preview_port == i, p.name).clicked() {
                            st.preview_port = i;
                        }
                    }
                }
            }
            if status == Status::Stale && cached.is_some() {
                ui.label(RichText::new("· out of date: run to refresh").size(11.5).color(t.muted));
            }
            if let Some(Ok(o)) = &cached {
                ui.label(RichText::new(format!("· {:.0} ms", o.ms)).size(11.5).color(t.muted));
            }
        });
        ui.add_space(4.0);
        match cached {
            None => {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Not run yet").color(t.muted));
                    if ui.button("Run this node").clicked() {
                        st.run_request = Some(RunRequest::Node(node.id));
                    }
                });
            }
            Some(Err(e)) => {
                egui::Frame::new().fill(t.danger.gamma_multiply(0.08)).stroke(egui::Stroke::new(1.0, t.danger.gamma_multiply(0.4))).corner_radius(6.0).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(e).color(t.danger));
                });
            }
            Some(Ok(out)) => {
                let port = if out.table(st.preview_port).is_some() { st.preview_port } else { out.ports.iter().position(|p| p.table().is_some()).unwrap_or(0) };
                let has_summary = !out.summary.metrics.is_empty() || out.summary.scatter.is_some() || out.summary.bars.is_some() || out.summary.heatmap.is_some() || !out.summary.tables.is_empty() || !out.summary.log.trim().is_empty();
                let avail = ui.available_size();
                let table = out.table(port).cloned();
                let summary_w = if has_summary { if table.is_some() { (avail.x * 0.45).clamp(320.0, 620.0) } else { avail.x } } else { 0.0 };
                ui.horizontal_top(|ui| {
                    if let Some(tb) = &table {
                        ui.vertical(|ui| {
                            ui.set_width((avail.x - summary_w - 14.0).max(200.0));
                            ui.label(RichText::new(format!("{} rows × {} columns", vizardcraft_core::numfmt::format(tb.rows.len() as f64, "#,0"), tb.columns.len())).size(11.5).color(t.muted));
                            preview::data_grid(ui, t, tb, ("flow_preview", node.id, port), (avail.y - 24.0).max(80.0));
                        });
                    } else if let Some(m) = out.ports.iter().find_map(|p| if let flow::PortData::Model(m) = p { Some(m.clone()) } else { None }) {
                        let _ = m;
                    }
                    if has_summary {
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.set_width(summary_w);
                            egui::ScrollArea::vertical().id_salt(("summary_scroll", node.id)).max_height(avail.y).auto_shrink([false, false]).show(ui, |ui| {
                                preview::summary(ui, t, &out.summary, node.id);
                            });
                        });
                    }
                });
            }
        }
    });
}
