//! Power Query: Get Data dialogs, the Tableau-style data source canvas, and the Power Query
//! Editor (queries, applied steps, transforms, M advanced editor, profiling).
//!
//! The editor fills the central area (`app.pq_open`): its own ribbon (Home, Transform, Add
//! Column, View, Tools, Help), the Queries pane on the left, the formula bar and data preview
//! with column quality / distribution / profile in the middle, Query Settings with the applied
//! steps on the right. Every action appends an M step to the query (`Table.SelectRows(#"Prev",
//! …)`), so the Advanced Editor always shows the real query. Every change is undoable
//! (`app.checkpoint`). Close & Apply syncs the model tables and refreshes.
//!
//! Previews are cached per (query, step, version of the queries): M is only re-evaluated when
//! something changed.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use egui::{RichText, Ui};
use vizcraft_query::profile::ColumnProfile;
use vizcraft_query::steps::{self, LoadTo, QueryDef, Step, steps_of};
use vizcraft_query::{Table, Value};

use crate::app::VizApp;
use crate::theme::Tokens;

pub mod canvas;
mod deps;
pub mod dialogs;
mod forms;
pub mod getdata;
mod grid;
pub mod icons;
pub mod ops;
mod panes;

pub use dialogs::Dialog;

/// Which page the Power Query area shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Editor,
    /// The data source canvas (joins and unions).
    Canvas,
}

/// View options (the View tab).
#[derive(Clone, Copy)]
pub struct ViewOpts {
    pub formula_bar: bool,
    pub quality: bool,
    pub distribution: bool,
    pub profile: bool,
    /// Profile every row instead of the top 1000.
    pub all_rows: bool,
    pub monospace: bool,
    pub whitespace: bool,
    pub queries_pane: bool,
    pub settings_pane: bool,
}

impl Default for ViewOpts {
    fn default() -> Self {
        ViewOpts { formula_bar: true, quality: true, distribution: true, profile: true, all_rows: false, monospace: false, whitespace: false, queries_pane: true, settings_pane: true }
    }
}

/// Column From Examples in progress: the example texts typed per row.
#[derive(Clone, Default)]
pub struct Examples {
    pub name: String,
    /// Input columns (empty = all).
    pub inputs: Vec<String>,
    pub values: HashMap<usize, String>,
    pub suggestions: Vec<vizcraft_query::examples::Suggestion>,
    pub chosen: usize,
    /// The examples the suggestions were computed for.
    pub computed_for: Vec<(usize, String)>,
}

/// The preview of the query at the selected step.
pub struct Preview {
    pub table: Rc<Table>,
    /// A non-table result (a number, text), shown as a single value.
    pub scalar: Option<String>,
    pub error: Option<String>,
    /// Evaluation time in milliseconds.
    pub ms: f64,
}

/// A source used recently (Home › Recent Sources).
#[derive(Clone, Debug, PartialEq)]
pub struct Recent {
    pub kind: String,
    /// The file path, folder, URL or server.
    pub target: String,
    /// The database (server sources).
    pub extra: String,
}

#[derive(Default)]
pub struct PqState {
    /// The query open in the editor.
    pub query: String,
    /// The selected step (`None`: the last one).
    pub step: Option<String>,
    pub tab: usize,
    /// Selected columns of the preview.
    pub selected: Vec<String>,
    pub formula: String,
    formula_for: Option<(String, String)>,
    pub view: ViewOpts,
    pub dialog: Option<Dialog>,
    pub examples: Option<Examples>,
    pub deps: bool,
    pub page: Page,
    pub get_data: Option<getdata::GetData>,
    pub gallery: Option<getdata::Gallery>,
    pub recent: Vec<Recent>,
    pub canvas: canvas::Canvas,
    /// Query groups folded in the Queries pane.
    pub folded: Vec<String>,
    /// The open column filter menu.
    pub filter: Option<grid::FilterMenu>,
    /// Query edits not applied to the model yet (Close & Apply applies them).
    pub pending: bool,
    cache: Option<(PreviewKey, Rc<Preview>)>,
    profiles: HashMap<(usize, bool), ColumnProfile>,
    tables: HashMap<String, (u64, Result<Rc<Table>, String>)>,
    hook_done: bool,
}

type PreviewKey = (u64, String, Option<String>);

/// A fingerprint of everything a query's result depends on inside the project: the queries
/// and parameters (hashed), and the model generation (sheet edits rebuild the model).
pub fn fingerprint(app: &VizApp) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for q in &app.project.queries.queries {
        q.name.hash(&mut h);
        q.m.hash(&mut h);
    }
    for p in &app.project.queries.parameters {
        p.name.hash(&mut h);
        p.value.hash(&mut h);
    }
    for s in &app.project.sheets {
        s.name.hash(&mut h);
        s.cells.len().hash(&mut h);
    }
    app.generation.hash(&mut h);
    h.finish()
}

/// Evaluates M text against the project (other queries, parameters and sheets by name) and
/// the desktop connectors.
pub fn eval_m(app: &VizApp, m: &str) -> Result<Value, String> {
    let host = vizcraft_connectors::DesktopHost;
    let ph = vizcraft_core::build::ProjectHost::new(&app.project, &host);
    vizcraft_query::evaluate(m, &ph).map_err(|e| e.message())
}

/// A value as a table for the preview: lists become one column, records name/value rows.
pub fn as_table(v: Value) -> Result<Rc<Table>, Option<String>> {
    match v {
        Value::Table(t) => Ok(t),
        Value::List(l) => Ok(Rc::new(Table::new(vec!["List".into()], l.iter().map(|v| vec![v.clone()]).collect()))),
        Value::Record(r) => Ok(Rc::new(Table::new(vec!["Name".into(), "Value".into()], r.names.iter().zip(r.values.iter()).map(|(n, v)| vec![Value::text(n), v.clone()]).collect()))),
        v => Err(Some(v.display())),
    }
}

impl PqState {
    pub fn refresh_preview(&mut self) {
        self.cache = None;
        self.profiles.clear();
        self.tables.clear();
    }

    pub fn profile(&mut self, t: &Table, c: usize) -> ColumnProfile {
        let all = self.view.all_rows;
        self.profiles.entry((c, all)).or_insert_with(|| vizcraft_query::profile::profile(t, c, if all { usize::MAX } else { 1000 })).clone()
    }
}

/// The preview of the current query at the selected step (cached).
pub fn preview(app: &mut VizApp) -> Option<Rc<Preview>> {
    let q = app.project.queries.query(&app.pq.query)?.clone();
    let key: PreviewKey = (fingerprint(app), app.pq.query.clone(), app.pq.step.clone());
    if let Some((k, p)) = &app.pq.cache
        && *k == key
    {
        return Some(p.clone());
    }
    let m = match &app.pq.step {
        Some(s) => steps::m_at_step(&q.m, s).unwrap_or_else(|_| q.m.clone()),
        None => q.m.clone(),
    };
    let t0 = std::time::Instant::now();
    let r = eval_m(app, &m);
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    let p = Rc::new(match r {
        Ok(v) => match as_table(v) {
            Ok(t) => Preview { table: t, scalar: None, error: None, ms },
            Err(s) => Preview { table: Rc::default(), scalar: s, error: None, ms },
        },
        Err(e) => Preview { table: Rc::default(), scalar: None, error: Some(e), ms },
    });
    app.pq.cache = Some((key, p.clone()));
    app.pq.profiles.clear();
    Some(p)
}

/// The preview table (empty when the step failed).
pub fn preview_table(app: &mut VizApp) -> Rc<Table> {
    preview(app).map(|p| p.table.clone()).unwrap_or_default()
}

/// A query's or data sheet's result, by name (cached per project version).
pub fn table_of(app: &mut VizApp, name: &str) -> Result<Rc<Table>, String> {
    let fp = fingerprint(app);
    if let Some((k, r)) = app.pq.tables.get(name)
        && *k == fp
    {
        return r.clone();
    }
    let r = if app.project.queries.query(name).is_some() {
        let host = vizcraft_connectors::DesktopHost;
        vizcraft_core::build::evaluate_query(&app.project, name, &host).map(Rc::new).map_err(|e| e.to_string())
    } else if let Some(s) = app.project.sheet(name) {
        Ok(Rc::new(vizcraft_core::sheet::evaluate(s).to_m()))
    } else {
        Err(format!("There's no query or sheet named {name}."))
    };
    app.pq.tables.insert(name.to_string(), (fp, r.clone()));
    r
}

// ---------------------------------------------------------------- editing queries

/// The steps of the current query.
pub fn steps(app: &VizApp) -> Vec<Step> {
    app.project.queries.query(&app.pq.query).and_then(|q| steps_of(&q.m).ok()).unwrap_or_default()
}

/// Replaces the current query's M (undoable).
pub fn set_m(app: &mut VizApp, label: &str, m: String) {
    let name = app.pq.query.clone();
    if app.project.queries.query(&name).is_some_and(|q| q.m == m) {
        return;
    }
    app.checkpoint(label);
    if let Some(q) = app.project.queries.query_mut(&name) {
        q.m = m;
    }
    app.pq.pending = true;
}

/// Adds a step after the selected one. `expr` uses `{prev}` for the previous step.
pub fn add_step(app: &mut VizApp, name: &str, expr: &str) {
    let Some(m) = app.project.queries.query(&app.pq.query).map(|q| q.m.clone()) else {
        app.error("Open a query first.");
        return;
    };
    let after = app.pq.step.clone();
    match steps::add_step(&m, after.as_deref(), name, |p| expr.replace("{prev}", p)) {
        Ok((m2, new)) => {
            let last = steps_of(&m2).ok().and_then(|s| s.last().map(|s| s.name.clone()));
            set_m(app, name, m2);
            app.pq.step = if Some(&new) == last.as_ref() { None } else { Some(new) };
        }
        Err(e) => app.error(e.message()),
    }
}

/// Applies a step edit (`steps::delete_step`, `rename_step`, …) to the current query.
pub fn edit_steps(app: &mut VizApp, label: &str, f: impl FnOnce(&str) -> vizcraft_query::Result<String>) -> bool {
    let Some(m) = app.project.queries.query(&app.pq.query).map(|q| q.m.clone()) else { return false };
    match f(&m) {
        Ok(m2) => {
            set_m(app, label, m2);
            true
        }
        Err(e) => {
            app.error(e.message());
            false
        }
    }
}

/// Creates a query (undoable) and returns its name. `load` puts it in the model.
pub fn new_query(app: &mut VizApp, base: &str, m: String, load: bool, group: Option<String>) -> String {
    let base = if base.trim().is_empty() { "Query" } else { base.trim() };
    let name = app.project.free_table_name(base);
    app.checkpoint(&format!("New query {name}"));
    app.project.queries.queries.push(QueryDef { name: name.clone(), m, load: if load { LoadTo::Model } else { LoadTo::None }, loaded_table: None, group, description: String::new() });
    app.pq.pending = true;
    name
}

/// Loads new queries into the model now (Get Data › Load).
pub fn apply_to_model(app: &mut VizApp) {
    vizcraft_core::build::sync_model_tables(&mut app.project);
    app.mark_data_dirty();
    app.pq.pending = false;
    app.pq.refresh_preview();
}

pub fn duplicate_query(app: &mut VizApp, name: &str) -> Option<String> {
    let q = app.project.queries.query(name)?.clone();
    let n = new_query(app, &q.name, q.m.clone(), q.load != LoadTo::None, q.group.clone());
    if let Some(d) = app.project.queries.query_mut(&n) {
        d.description = q.description;
    }
    Some(n)
}

pub fn reference_query(app: &mut VizApp, name: &str) -> Option<String> {
    let q = app.project.queries.query(name)?.clone();
    Some(new_query(app, &format!("{} (2)", q.name), format!("let\n    Source = {}\nin\n    Source", steps::ident(&q.name)), true, q.group))
}

pub fn delete_query(app: &mut VizApp, name: &str) {
    let users: Vec<String> = app.project.queries.queries.iter().filter(|q| q.name != name && app.project.queries.dependencies(&q.name).iter().any(|d| d == name)).map(|q| q.name.clone()).collect();
    if !users.is_empty() {
        app.error(format!("{name} is used by {}. Remove those references first.", users.join(", ")));
        return;
    }
    app.checkpoint(&format!("Delete query {name}"));
    app.project.queries.queries.retain(|q| q.name != name);
    vizcraft_core::build::sync_model_tables(&mut app.project);
    app.pq.pending = true;
    if app.pq.query == name {
        app.pq.query = app.project.queries.queries.first().map(|q| q.name.clone()).unwrap_or_default();
        app.pq.step = None;
        app.pq.selected.clear();
    }
}

/// Renames a query: references in other queries and the model table follow.
pub fn rename_query(app: &mut VizApp, old: &str, new: &str) -> Result<(), String> {
    let new = new.trim();
    if new.is_empty() {
        return Err("A query needs a name.".into());
    }
    if new == old {
        return Ok(());
    }
    if !new.eq_ignore_ascii_case(old) && app.project.free_table_name(new) != new {
        return Err(format!("There is already a query, sheet or table named {new}."));
    }
    app.checkpoint(&format!("Rename query {old}"));
    for q in app.project.queries.queries.iter_mut() {
        if q.name == old {
            q.name = new.to_string();
        } else {
            q.m = steps::rename_refs(&q.m, old, new);
        }
    }
    let p = &mut app.project;
    for t in p.model.tables.iter_mut() {
        if t.source == vizcraft_core::doc::TableSource::Query(old.to_string()) {
            t.source = vizcraft_core::doc::TableSource::Query(new.to_string());
            if t.name == old {
                t.name = new.to_string();
            }
        }
    }
    for r in p.model.relationships.iter_mut() {
        if r.from_table == old {
            r.from_table = new.to_string();
        }
        if r.to_table == old {
            r.to_table = new.to_string();
        }
    }
    for m in p.model.measures.iter_mut() {
        if m.table == old {
            m.table = new.to_string();
        }
    }
    if app.pq.query == old {
        app.pq.query = new.to_string();
    }
    app.pq.pending = true;
    Ok(())
}

pub fn set_load(app: &mut VizApp, name: &str, on: bool) {
    app.checkpoint(if on { "Enable load" } else { "Disable load" });
    if let Some(q) = app.project.queries.query_mut(name) {
        q.load = if on { LoadTo::Model } else { LoadTo::None };
    }
    app.pq.pending = true;
}

pub fn set_group(app: &mut VizApp, name: &str, group: Option<String>) {
    app.checkpoint("Move to group");
    if let Some(q) = app.project.queries.query_mut(name) {
        q.group = group.filter(|g| !g.trim().is_empty());
    }
}

/// Remembers a source for Home › Recent Sources.
pub fn remember(app: &mut VizApp, kind: &str, target: &str, extra: &str) {
    let r = Recent { kind: kind.to_string(), target: target.to_string(), extra: extra.to_string() };
    app.pq.recent.retain(|x| *x != r);
    app.pq.recent.insert(0, r);
    app.pq.recent.truncate(12);
}

// ---------------------------------------------------------------- public entry points

/// Opens the editor on a query (or the current / first one).
pub fn open_editor(app: &mut VizApp, query: Option<&str>) {
    app.pq_open = true;
    app.pq.page = Page::Editor;
    match query {
        Some(q) => {
            app.pq.query = q.to_string();
            app.pq.step = None;
            app.pq.selected.clear();
            app.pq.examples = None;
        }
        None if app.project.queries.query(&app.pq.query).is_none() => {
            app.pq.query = app.project.queries.queries.first().map(|q| q.name.clone()).unwrap_or_default();
            app.pq.step = None;
        }
        None => {}
    }
}

/// The Tableau-style data source page: drag tables onto a canvas to join / union them.
pub fn data_source_canvas(app: &mut VizApp) {
    app.pq_open = true;
    app.pq.page = Page::Canvas;
}

/// Opens the Get Data flow for a source kind: "csv", "text", "excel", "json", "xml", "folder",
/// "web", "sqlite", "postgresql", "mysql", "sqlserver", "odbc", "parquet", "odata", "blank", "enter", "more".
pub fn get_data(app: &mut VizApp, kind: &str) {
    getdata::open(app, kind);
}

/// Close & Apply: the model picks up the queries that load, and everything refreshes.
pub fn close_and_apply(app: &mut VizApp) {
    apply_to_model(app);
    app.pq_open = false;
    app.toast("Applied query changes");
}

fn tab_button(ui: &mut Ui, t: &Tokens, label: &str, active: bool) -> bool {
    let text = if active { RichText::new(label).strong().size(12.5).color(t.accent) } else { RichText::new(label).size(12.5).color(t.text) };
    let r = ui.add(egui::Button::new(text).frame(false).min_size(egui::vec2(56.0, 24.0)));
    if active {
        let rect = r.rect;
        ui.painter().line_segment([egui::pos2(rect.left() + 6.0, rect.bottom() + 1.0), egui::pos2(rect.right() - 6.0, rect.bottom() + 1.0)], egui::Stroke::new(2.5, t.accent));
    }
    r.clicked()
}

/// `VIZCRAFT_OPEN=<dialog>` opens a dialog once at start (demos and screenshots).
fn demo_hook(app: &mut VizApp) {
    if app.pq.hook_done {
        return;
    }
    app.pq.hook_done = true;
    let Ok(all) = std::env::var("VIZCRAFT_OPEN") else { return };
    for v in all.split(';') {
        demo_one(app, v);
    }
}

fn demo_one(app: &mut VizApp, v: &str) {
    let (what, arg) = v.split_once(':').unwrap_or((v, ""));
    match what {
        "get-data" => get_data(app, if arg.is_empty() { "csv" } else { arg }),
        "canvas" => {
            data_source_canvas(app);
            canvas::demo(app);
        }
        "merge" => {
            open_editor(app, None);
            app.pq.dialog = Some(Dialog::merge(app, false));
        }
        "append" => {
            open_editor(app, None);
            app.pq.dialog = Some(Dialog::append(false));
        }
        "advanced" => {
            open_editor(app, None);
            app.pq.dialog = Some(Dialog::advanced(app));
        }
        "conditional" => {
            open_editor(app, None);
            app.pq.dialog = Some(Dialog::conditional(app));
        }
        "custom" => {
            open_editor(app, None);
            app.pq.dialog = Some(Dialog::custom_column(app));
        }
        "group" => {
            open_editor(app, None);
            app.pq.dialog = Some(Dialog::group_by(app));
        }
        "params" => {
            open_editor(app, None);
            app.pq.dialog = Some(Dialog::parameters(app, None));
        }
        "deps" => {
            open_editor(app, None);
            app.pq.deps = true;
        }
        "tab" => {
            open_editor(app, None);
            app.pq.tab = arg.parse().unwrap_or(0);
        }
        "query" => open_editor(app, Some(arg)),
        "col" => {
            open_editor(app, None);
            app.pq.selected = vec![arg.to_string()];
        }
        "filter" => {
            open_editor(app, None);
            let t = preview_table(app);
            app.pq.selected = vec![arg.to_string()];
            app.pq.filter = Some(grid::new_filter(&t, arg, egui::pos2(700.0, 230.0)));
        }
        "examples" => {
            open_editor(app, None);
            app.pq.examples = Some(Examples { name: "Custom".into(), ..Default::default() });
        }
        _ => {}
    }
}

/// Shows the Power Query area (fills the central area).
pub fn show(app: &mut VizApp, ui: &mut Ui) {
    let t = app.tokens();
    if app.pq.page == Page::Canvas {
        canvas::show(app, ui, &t);
        return;
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) && app.pq.dialog.is_none() && app.pq.examples.is_none() && app.pq.get_data.is_none() && app.pq.filter.is_none() {
        app.pq_open = false;
        return;
    }
    if app.project.queries.query(&app.pq.query).is_none() {
        app.pq.query = app.project.queries.queries.first().map(|q| q.name.clone()).unwrap_or_default();
        app.pq.step = None;
    }
    let steps = steps(app);
    if let Some(s) = &app.pq.step
        && !steps.iter().any(|x| &x.name == s)
    {
        app.pq.step = None;
    }
    egui::Panel::top("pq_ribbon").frame(egui::Frame::new().fill(t.ribbon).inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 4 }).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
            icons::paint(ui.painter(), r, "transform", t.accent, t.dark);
            ui.label(RichText::new("Power Query Editor").strong().size(13.5).color(t.text));
            ui.add_space(14.0);
            for (i, name) in ["Home", "Transform", "Add Column", "View", "Tools", "Help"].iter().enumerate() {
                if tab_button(ui, &t, name, app.pq.tab == i) {
                    app.pq.tab = i;
                }
                ui.add_space(4.0);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new("Data source canvas").min_size(egui::vec2(0.0, 24.0))).on_hover_text("Join and union tables by dragging them onto a canvas (Tableau style)").clicked() {
                    app.pq.page = Page::Canvas;
                }
                if app.pq.pending {
                    ui.label(RichText::new("Changes not applied").size(11.0).color(egui::Color32::from_rgb(0xB4, 0x53, 0x09)))
                        .on_hover_text("The model still uses the queries as they were. Close & Apply loads the changes.");
                }
            });
        });
        ui.add_space(2.0);
        ops::ribbon(app, ui, &t);
    });
    egui::Panel::bottom("pq_status").exact_size(24.0).frame(egui::Frame::new().fill(t.panel_alt).inner_margin(egui::Margin::symmetric(10, 3)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
        panes::status_bar(app, ui, &t);
    });
    if app.pq.view.queries_pane {
        egui::Panel::left("pq_queries").resizable(true).default_size(230.0).min_size(160.0).max_size(420.0).frame(egui::Frame::new().fill(t.panel).inner_margin(8).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
            panes::queries(app, ui, &t);
        });
    }
    if app.pq.view.settings_pane {
        egui::Panel::right("pq_settings").resizable(true).default_size(260.0).min_size(190.0).max_size(420.0).frame(egui::Frame::new().fill(t.panel).inner_margin(8).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| {
            panes::settings(app, ui, &t, &steps);
        });
    }
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg).inner_margin(0)).show(ui, |ui| {
        if app.project.queries.queries.is_empty() {
            panes::empty(app, ui, &t);
            return;
        }
        if app.pq.view.formula_bar {
            panes::formula_bar(app, ui, &t, &steps);
        }
        grid::show(app, ui, &t);
    });
    if app.pq.deps {
        deps::show(app, ui.ctx(), &t);
    }
}

/// Floating dialogs (Get Data, connection settings, editor dialogs). Drawn also when the
/// editor is closed: Get Data opens from the main ribbon.
pub fn dialogs(app: &mut VizApp, ctx: &egui::Context) {
    demo_hook(app);
    getdata::show(app, ctx);
    dialogs::show(app, ctx);
}
