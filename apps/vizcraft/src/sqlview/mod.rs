//! The SQL view: an embedded DuckDB workbench over the semantic model, files, attached
//! databases and the project warehouse — object explorer, multi-tab editor with highlighting
//! and autocomplete, results grids, plans, history, snippets, a Python (pandas) tab, and the
//! bridges back into the report: "Load to model" and "Create chart" (Power Query queries built
//! on `VizCraft.Sql(...)`).

mod editor;
mod grid;
mod store;

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::ops::Range;
use std::path::{Path, PathBuf};

use egui::{Color32, FontId, RichText, Ui, vec2};
use vizcraft_core::build::{Built, ColType, TableData};
use vizcraft_core::doc::*;
use vizcraft_query::m::eval::Host;
use vizcraft_query::steps::{LoadTo, QueryDef};
use vizcraft_sql::python::{PythonOutput, PythonRun};
use vizcraft_sql::worker::{Job, Worker};
use vizcraft_sql::{CatalogObject, Engine, ObjectKind, QueryResult, StatementOutput, text};

use crate::app::VizApp;
use crate::theme::Tokens;
use editor::{Autocomplete, Completion, Lang};
use grid::GridState;
use store::{HistoryEntry, Snippet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabKind {
    Sql,
    Python,
}

pub struct Message {
    pub ok: bool,
    pub text: String,
}

pub struct EditorTab {
    pub uid: u64,
    pub title: String,
    pub kind: TabKind,
    pub text: String,
    pub outputs: Vec<StatementOutput>,
    pub messages: Vec<Message>,
    /// Selected result tab: an index into `outputs`, or `usize::MAX` for messages.
    pub result_tab: usize,
    grids: Vec<GridState>,
    /// The running job and when it started.
    running: Option<(u64, f64)>,
    cursor: usize,
    selection: Option<Range<usize>>,
    ac: Autocomplete,
    py_run: Option<PythonRun>,
    py_out: Option<PythonOutput>,
    py_grid: GridState,
    /// Python: show the input `df` instead of the output.
    py_show_input: bool,
}

impl EditorTab {
    fn new(uid: u64, kind: TabKind, title: String, text: String) -> EditorTab {
        EditorTab {
            uid,
            title,
            kind,
            text,
            outputs: vec![],
            messages: vec![],
            result_tab: 0,
            grids: vec![],
            running: None,
            cursor: 0,
            selection: None,
            ac: Autocomplete::default(),
            py_run: None,
            py_out: None,
            py_grid: GridState::default(),
            py_show_input: false,
        }
    }
    fn editor_id(&self) -> egui::Id {
        egui::Id::unique(("sql-editor", self.uid))
    }
    /// The SQL to run for "Run": the selection, else the statement under the cursor.
    fn current_statement(&self) -> String {
        if let Some(r) = &self.selection
            && let Some(s) = self.text.get(r.clone())
            && !s.trim().is_empty()
        {
            return s.to_string();
        }
        text::statement_at(&self.text, self.cursor).and_then(|r| self.text.get(r)).unwrap_or_default().to_string()
    }
    fn current_rows(&self) -> Option<&QueryResult> {
        match self.outputs.get(self.result_tab) {
            Some(StatementOutput::Rows(r)) if !r.columns.is_empty() => Some(r),
            _ => None,
        }
    }
}

struct Session {
    engine: Engine,
    worker: Option<Worker>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LeftTab {
    Objects,
    History,
    Snippets,
}

#[derive(Clone, Debug)]
enum NamePurpose {
    LoadToModel(String),
    SaveWarehouse(String),
    SaveSnippet(String),
    CreateChart(String),
    SavePythonOut,
}

enum Dialog {
    AttachDb { kind: String, target: String, alias: String, error: Option<String> },
    Name { purpose: NamePurpose, name: String },
}

/// Things the UI asked for, applied after drawing (they need the whole app).
enum Action {
    Run,
    RunAll,
    Explain(bool),
    Cancel,
    RunSqlInNewTab(String),
    NewTab(TabKind),
    CloseTab(usize),
    AttachFile,
    DetachFile(String),
    Drop(CatalogObject),
    LoadWarehouseTable(String),
    Export(String),
    OpenPython,
    RunPython,
    StopPython,
    Ask(NamePurpose, String),
    InsertText(String),
    OpenSql(String),
    RefreshCatalog,
    ClearHistory,
    DeleteSnippet(usize),
}

pub struct SqlUi {
    session: Option<Session>,
    session_error: Option<String>,
    /// The warehouse path the session was opened for.
    session_for: Option<Option<PathBuf>>,
    synced_generation: Option<u64>,
    pub tabs: Vec<EditorTab>,
    pub active: usize,
    next_uid: u64,
    catalog: Vec<CatalogObject>,
    catalog_stale: bool,
    left: LeftTab,
    filter: String,
    history: Option<Vec<HistoryEntry>>,
    snippets: Option<Vec<Snippet>>,
    /// The last result (the Python tab's `df`).
    pub last_result: Option<QueryResult>,
    pub row_cap: usize,
    dialog: Option<Dialog>,
    notices: Vec<(bool, String)>,
    demo_started: bool,
    version: String,
}

const WELCOME: &str = "-- VizCraft SQL (DuckDB). Model tables, files and your warehouse are all queryable.\n-- Ctrl+Enter runs the statement under the cursor (or the selection); F5 runs everything.\nSELECT st.Region,\n       COUNT(*)                         AS orders,\n       SUM(s.Quantity)                  AS units,\n       ROUND(SUM(s.Quantity * s.\"Unit Price\" * (1 - s.Discount)), 2) AS revenue\nFROM Sales s\nJOIN Store st USING (StoreID)\nGROUP BY ALL\nORDER BY revenue DESC;\n";

impl Default for SqlUi {
    fn default() -> Self {
        SqlUi {
            session: None,
            session_error: None,
            session_for: None,
            synced_generation: None,
            tabs: vec![EditorTab::new(1, TabKind::Sql, "Query 1".into(), WELCOME.into())],
            active: 0,
            next_uid: 2,
            catalog: vec![],
            catalog_stale: true,
            left: LeftTab::Objects,
            filter: String::new(),
            history: None,
            snippets: None,
            last_result: None,
            row_cap: 1000,
            dialog: None,
            notices: vec![],
            demo_started: false,
            version: String::new(),
        }
    }
}

fn warehouse_for(project: Option<&Path>) -> Option<PathBuf> {
    project.map(|p| p.with_extension("duckdb"))
}

impl SqlUi {
    /// Opens the engine for the project's warehouse (re-opens when the project path changes).
    fn ensure(&mut self, project: Option<&Path>) -> bool {
        let want = warehouse_for(project);
        if self.session_for.as_ref() != Some(&want) {
            self.session = None;
            self.session_for = Some(want.clone());
            self.synced_generation = None;
            for t in &mut self.tabs {
                t.running = None;
            }
            match Engine::open(want.as_deref()) {
                Ok(e) => {
                    self.version = e.version();
                    let worker = Worker::new(&e).map_err(|e| self.notices.push((false, e))).ok();
                    self.session = Some(Session { engine: e, worker });
                    self.session_error = None;
                    self.catalog_stale = true;
                }
                Err(e) => self.session_error = Some(e),
            }
        }
        self.session.is_some()
    }

    /// Registers the model's tables after a refresh.
    fn sync_model(&mut self, project: Option<&Path>, built: &Built, generation: u64) {
        if !self.ensure(project) || self.synced_generation == Some(generation) {
            return;
        }
        if let Some(s) = &mut self.session {
            for (t, e) in s.engine.sync_model(built.tables.values().map(|t| t.as_ref())) {
                self.notices.push((false, format!("SQL: couldn't register {t}: {e}")));
            }
        }
        self.synced_generation = Some(generation);
        self.catalog_stale = true;
    }

    fn refresh_catalog(&mut self) {
        if let Some(s) = &self.session {
            match s.engine.catalog() {
                Ok(c) => self.catalog = c,
                Err(e) => self.notices.push((false, e)),
            }
        }
        self.catalog_stale = false;
    }

    fn tab(&mut self) -> Option<&mut EditorTab> {
        let i = self.active.min(self.tabs.len().saturating_sub(1));
        self.tabs.get_mut(i)
    }

    fn add_tab(&mut self, kind: TabKind, text: String) -> usize {
        let n = self.tabs.iter().filter(|t| t.kind == kind).count() + 1;
        let title = match kind {
            TabKind::Sql => format!("Query {n}"),
            TabKind::Python => format!("Python {n}"),
        };
        self.tabs.push(EditorTab::new(self.next_uid, kind, title, text));
        self.next_uid += 1;
        self.active = self.tabs.len() - 1;
        self.active
    }

    fn submit(&mut self, job: Job) {
        let Some(worker) = self.session.as_mut().and_then(|s| s.worker.as_mut()) else {
            self.notices.push((false, self.session_error.clone().unwrap_or_else(|| "The SQL engine isn't running.".into())));
            return;
        };
        let id = worker.submit(job);
        if let Some(t) = self.tab() {
            t.running = Some((id, crate::app::now()));
        }
    }

    /// Collects finished jobs and Python runs.
    fn poll(&mut self, project_name: &str) {
        let done = match self.session.as_mut().and_then(|s| s.worker.as_mut()) {
            Some(w) => w.poll(),
            None => vec![],
        };
        for d in done {
            if let Job::Export { path, .. } = &d.job {
                match d.outputs.first() {
                    Some(StatementOutput::Error { message, .. }) => self.notices.push((false, format!("Export failed: {message}"))),
                    _ => self.notices.push((true, format!("Exported to {path}"))),
                }
                if let Some(t) = self.tabs.iter_mut().find(|t| t.running.is_some_and(|(id, _)| id == d.id)) {
                    t.running = None;
                }
                continue;
            }
            let mut ddl = false;
            let mut rows_total = 0;
            let mut error = None;
            if let Some(t) = self.tabs.iter_mut().find(|t| t.running.is_some_and(|(id, _)| id == d.id)) {
                t.running = None;
                t.messages.clear();
                for (i, o) in d.outputs.iter().enumerate() {
                    match o {
                        StatementOutput::Rows(r) => {
                            rows_total += r.rows.len();
                            let w = text::first_word(&r.sql).to_ascii_uppercase();
                            if matches!(w.as_str(), "CREATE" | "DROP" | "ALTER" | "ATTACH" | "DETACH" | "INSERT" | "COPY" | "USE" | "INSTALL" | "LOAD" | "UPDATE" | "DELETE") {
                                ddl = true;
                            }
                            let what = match (r.columns.is_empty(), r.rows_affected) {
                                (_, Some(n)) => format!("{n} rows affected"),
                                (true, None) => "done".into(),
                                (false, None) => format!("{} row{}{}", r.rows.len(), if r.rows.len() == 1 { "" } else { "s" }, if r.truncated { " (truncated)" } else { "" }),
                            };
                            t.messages.push(Message { ok: true, text: format!("Statement {}: {what} in {:.1} ms\n{}", i + 1, r.elapsed_ms, one_line(&r.sql, 160)) });
                        }
                        StatementOutput::Plan { sql, elapsed_ms, .. } => t.messages.push(Message { ok: true, text: format!("Statement {}: plan in {elapsed_ms:.1} ms\n{}", i + 1, one_line(sql, 160)) }),
                        StatementOutput::Error { sql, message } => {
                            error = Some(message.clone());
                            t.messages.push(Message { ok: false, text: format!("Statement {}: {message}\n{}", i + 1, one_line(sql, 160)) });
                        }
                    }
                }
                t.messages.push(Message { ok: error.is_none(), text: format!("Finished in {:.1} ms.", d.elapsed_ms) });
                t.grids = d.outputs.iter().map(|_| GridState::default()).collect();
                // Show the last result set (or plan); errors show the messages.
                t.result_tab = if error.is_some() {
                    usize::MAX
                } else {
                    d.outputs
                        .iter()
                        .rposition(|o| matches!(o, StatementOutput::Rows(r) if !r.columns.is_empty()) || matches!(o, StatementOutput::Plan { .. }))
                        .unwrap_or(usize::MAX)
                };
                t.outputs = d.outputs.clone();
                if let Some(r) = t.current_rows() {
                    self.last_result = Some(r.clone());
                }
            }
            let sql = match &d.job {
                Job::Script { sql, .. } | Job::Explain { sql, .. } => sql.clone(),
                Job::Export { sql, .. } => sql.clone(),
            };
            let e = HistoryEntry { at: crate::app::now(), sql, ms: d.elapsed_ms, rows: error.is_none().then_some(rows_total), error, project: project_name.to_string() };
            store::append_history(&e);
            if let Some(h) = &mut self.history {
                h.insert(0, e);
            }
            if ddl {
                self.catalog_stale = true;
            }
        }
        for t in &mut self.tabs {
            if let Some(run) = &mut t.py_run
                && let Some(out) = run.try_finish()
            {
                t.py_run = None;
                t.py_grid = GridState::default();
                t.py_show_input = false;
                t.py_out = Some(out);
            }
        }
    }

    fn busy(&self) -> bool {
        self.tabs.iter().any(|t| t.running.is_some() || t.py_run.is_some())
    }

}

/// Completion candidates for a word (and a `table.` qualifier).
fn completions(catalog: &[CatalogObject], src: &str, word: &str, qualifier: Option<&str>) -> Vec<Completion> {
    {
        let w = word.to_lowercase();
        let matches = |s: &str| w.is_empty() || s.to_lowercase().starts_with(&w);
        let mut out: Vec<Completion> = Vec::new();
        if let Some(q) = qualifier {
            let table = resolve_alias(src, q, &catalog).unwrap_or_else(|| q.to_string());
            if let Some(o) = catalog.iter().find(|o| o.name.eq_ignore_ascii_case(&table)) {
                for (c, ty) in &o.columns {
                    if matches(c) {
                        out.push(Completion { text: c.clone(), kind: "column", detail: ty.to_lowercase() });
                    }
                }
            }
            return out;
        }
        for o in catalog.iter() {
            if matches(&o.name) {
                out.push(Completion { text: o.name.clone(), kind: "table", detail: o.kind.label().trim_end_matches('s').to_lowercase() });
            }
        }
        let mut seen = BTreeSet::new();
        // Columns of tables the statement mentions first.
        let mentioned: Vec<&CatalogObject> = catalog.iter().filter(|o| src.to_lowercase().contains(&o.name.to_lowercase())).collect();
        for o in mentioned.iter().copied().chain(catalog.iter()) {
            for (c, _) in &o.columns {
                if matches(c) && seen.insert(c.to_lowercase()) {
                    out.push(Completion { text: c.clone(), kind: "column", detail: o.name.clone() });
                }
            }
        }
        for f in text::FUNCTIONS {
            if matches(f) {
                out.push(Completion { text: f.to_string(), kind: "function", detail: "function".into() });
            }
        }
        for k in text::KEYWORDS {
            if matches(k) {
                out.push(Completion { text: k.to_string(), kind: "keyword", detail: String::new() });
            }
        }
        out.truncate(40);
        out
    }
}

/// `s` in `FROM Sales s` → `Sales`.
fn resolve_alias(src: &str, alias: &str, catalog: &[CatalogObject]) -> Option<String> {
    let toks: Vec<String> = text::tokenize(src)
        .into_iter()
        .filter(|(_, k)| matches!(k, text::Tok::Ident | text::Tok::QuotedIdent | text::Tok::Keyword))
        .map(|(r, _)| src.get(r).unwrap_or_default().trim_matches('"').to_string())
        .collect();
    for i in 0..toks.len() {
        let name = &toks[i];
        if !catalog.iter().any(|o| o.name.eq_ignore_ascii_case(name)) {
            continue;
        }
        let mut j = i + 1;
        if toks.get(j).is_some_and(|t| t.eq_ignore_ascii_case("as")) {
            j += 1;
        }
        if toks.get(j).is_some_and(|t| t.eq_ignore_ascii_case(alias)) {
            return Some(name.clone());
        }
    }
    None
}

fn one_line(s: &str, max: usize) -> String {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max { flat.chars().take(max).collect::<String>() + "…" } else { flat }
}

/// SQL as an M text literal inside `VizCraft.Sql(...)`.
pub fn m_for_sql(sql: &str) -> String {
    let lit = sql.trim().trim_end_matches(';').trim().replace('"', "\"\"").replace("#(", "#(#)(").replace("\r\n", "\n").replace('\r', "").replace('\n', "#(lf)");
    format!("let\n    Source = VizCraft.Sql(\"{lit}\")\nin\n    Source")
}

// ---------------------------------------------------------------- Power Query host

/// The Power Query host the app refreshes with: the desktop connectors plus `VizCraft.Sql`
/// evaluated on the embedded engine. Model tables a SQL query refers to that aren't registered
/// yet (the first refresh) are loaded on demand.
pub struct SqlHost<'a> {
    inner: &'a dyn Host,
    ui: RefCell<&'a mut SqlUi>,
    project: &'a Project,
    prev: &'a Built,
    path: Option<&'a Path>,
    depth: Cell<usize>,
}

impl<'a> SqlHost<'a> {
    pub fn new(inner: &'a dyn Host, ui: &'a mut SqlUi, project: &'a Project, prev: &'a Built, path: Option<&'a Path>) -> SqlHost<'a> {
        SqlHost { inner, ui: RefCell::new(ui), project, prev, path, depth: Cell::new(0) }
    }
}

impl Host for SqlHost<'_> {
    fn resolve(&self, name: &str) -> Option<vizcraft_query::Result<vizcraft_query::Value>> {
        self.inner.resolve(name)
    }
    fn current_workbook(&self) -> vizcraft_query::Result<vizcraft_query::Table> {
        self.inner.current_workbook()
    }
    fn file_contents(&self, path: &str) -> vizcraft_query::Result<Vec<u8>> {
        self.inner.file_contents(path)
    }
    fn excel_workbook(&self, bytes: &[u8], use_headers: bool) -> vizcraft_query::Result<vizcraft_query::Table> {
        self.inner.excel_workbook(bytes, use_headers)
    }
    fn folder_files(&self, path: &str) -> vizcraft_query::Result<vizcraft_query::Table> {
        self.inner.folder_files(path)
    }
    fn web_contents(&self, url: &str) -> vizcraft_query::Result<Vec<u8>> {
        self.inner.web_contents(url)
    }
    fn database(&self, kind: &str, args: &[String], query: Option<&str>) -> vizcraft_query::Result<vizcraft_query::Table> {
        self.inner.database(kind, args, query)
    }
    fn sql(&self, query: &str) -> vizcraft_query::Result<vizcraft_query::Table> {
        let ev = |m: String| vizcraft_query::Error::Eval(m);
        if !self.ui.borrow_mut().ensure(self.path) {
            return Err(ev(self.ui.borrow().session_error.clone().unwrap_or_else(|| "The SQL engine isn't available.".into())));
        }
        // Model tables the query mentions that the engine doesn't have yet.
        let words: BTreeSet<String> = text::tokenize(query)
            .into_iter()
            .filter(|(_, k)| matches!(k, text::Tok::Ident | text::Tok::QuotedIdent))
            .map(|(r, _)| query.get(r).unwrap_or_default().trim_matches('"').to_lowercase())
            .collect();
        for mt in &self.project.model.tables {
            if !words.contains(&mt.name.to_lowercase()) {
                continue;
            }
            let registered = self.ui.borrow().session.as_ref().is_some_and(|s| s.engine.is_registered(&mt.name));
            if registered {
                continue;
            }
            let data: Option<TableData> = if let Some(d) = self.prev.tables.get(&mt.name) {
                Some((**d).clone())
            } else {
                match &mt.source {
                    TableSource::Query(q) if self.depth.get() < 6 => {
                        self.depth.set(self.depth.get() + 1);
                        let r = vizcraft_core::build::evaluate_query(self.project, q, self);
                        self.depth.set(self.depth.get().saturating_sub(1));
                        r.ok().map(|t| {
                            let mut d = TableData::from_m(&mt.name, &t);
                            d.normalize();
                            d
                        })
                    }
                    TableSource::Sheet(s) => self.project.sheet(s).map(vizcraft_core::sheet::evaluate),
                    _ => None,
                }
            };
            if let Some(mut d) = data {
                d.name = mt.name.clone();
                if let Some(s) = self.ui.borrow_mut().session.as_mut() {
                    let _ = s.engine.register_table(&d);
                }
            }
        }
        let ui = self.ui.borrow();
        let s = ui.session.as_ref().ok_or_else(|| ev("The SQL engine isn't available.".into()))?;
        s.engine.query(query, usize::MAX).map(|r| r.to_m()).map_err(ev)
    }
}

// ---------------------------------------------------------------- public entry points

/// Get data → "VizCraft SQL (DuckDB)": a Power Query query that starts from SQL over the model
/// and the warehouse. Opens it in the Power Query Editor.
pub fn get_data_sql(app: &mut VizApp) {
    let table = app.project.model.tables.iter().find(|t| !t.hidden && !matches!(t.source, TableSource::Parameter(_))).map(|t| t.name.clone());
    let sql = match table {
        Some(t) => format!("SELECT * FROM {} LIMIT 1000", editor::quote_if_needed(&t)),
        None => "SELECT 42 AS answer".into(),
    };
    let name = app.project.queries.free_name("SQL query");
    app.checkpoint("New SQL query");
    app.project.queries.queries.push(QueryDef { name: name.clone(), m: m_for_sql(&sql), load: LoadTo::Model, loaded_table: None, group: Some("SQL".into()), description: "VizCraft.Sql over the model and the warehouse (DuckDB)".into() });
    app.mark_data_dirty();
    crate::pq::open_editor(app, Some(&name));
}

/// Opens the SQL view.
pub fn open(app: &mut VizApp) {
    app.view = crate::app::View::Sql;
    app.pq_open = false;
}

pub fn dialogs(app: &mut VizApp, ctx: &egui::Context) {
    let _ = (app, ctx);
}

/// The SQL view.
pub fn show(app: &mut VizApp, ui: &mut Ui) {
    let t = app.tokens();
    let path = app.path.clone();
    let generation = app.generation;
    app.sql_ui.sync_model(path.as_deref(), &app.built, generation);
    let pname = app.project.name.clone();
    app.sql_ui.poll(&pname);
    demo_python(app);
    if app.sql_ui.catalog_stale {
        app.sql_ui.refresh_catalog();
    }
    for (ok, msg) in std::mem::take(&mut app.sql_ui.notices) {
        if ok { app.toast(msg) } else { app.error(msg) }
    }
    let mut actions: Vec<Action> = Vec::new();
    demo_hook(app, &mut actions);
    // Shortcuts: Ctrl/Cmd+Enter runs the statement, F5 runs all, Esc cancels.
    let (run, run_all) = ui.input_mut(|i| {
        (
            i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Enter)),
            i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::F5)),
        )
    });
    let is_sql = app.sql_ui.tabs.get(app.sql_ui.active).is_some_and(|t| t.kind == TabKind::Sql);
    if run {
        actions.push(if is_sql { Action::Run } else { Action::RunPython });
    }
    if run_all {
        actions.push(if is_sql { Action::RunAll } else { Action::RunPython });
    }

    egui::Panel::left("sql-explorer")
        .resizable(true)
        .default_size(272.0)
        .size_range(200.0..=520.0)
        .frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::same(10)).stroke(egui::Stroke::new(1.0, t.border)))
        .show(ui, |ui| explorer(app, ui, &t, &mut actions));
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ui, |ui| {
        tab_bar(app, ui, &t, &mut actions);
        toolbar(app, ui, &t, &mut actions);
        let h = ui.available_height();
        egui::Panel::bottom("sql-results")
            .resizable(true)
            .default_size((h * 0.48).max(160.0))
            .size_range(90.0..=(h - 80.0).max(120.0))
            .frame(egui::Frame::new().fill(t.panel).stroke(egui::Stroke::new(1.0, t.border)))
            .show(ui, |ui| results(app, ui, &t, &mut actions));
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.panel)).show(ui, |ui| editor_area(app, ui, &t));
    });
    dialog(app, ui.ctx(), &t, &mut actions);
    for a in actions {
        apply(app, a, ui.ctx());
    }
    if app.sql_ui.busy() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(80));
    }
}

/// `VIZCRAFT_SQL_DEMO="select …"` puts SQL in the first tab and runs it; with
/// `VIZCRAFT_SQL_DEMO_PY="…"` a Python tab then runs over the result (screenshots, demos).
fn demo_hook(app: &mut VizApp, actions: &mut Vec<Action>) {
    if app.sql_ui.demo_started {
        return;
    }
    app.sql_ui.demo_started = true;
    if let Ok(sql) = std::env::var("VIZCRAFT_SQL_DEMO") {
        if let Some(tab) = app.sql_ui.tabs.first_mut() {
            if !sql.trim().is_empty() {
                tab.text = sql.replace("\\n", "\n");
            }
            tab.cursor = 0;
        }
        app.sql_ui.active = 0;
        actions.push(Action::RunAll);
    }
}

fn demo_python(app: &mut VizApp) {
    if let Ok(py) = std::env::var("VIZCRAFT_SQL_DEMO_PY")
        && app.sql_ui.last_result.is_some()
        && !app.sql_ui.tabs.iter().any(|t| t.kind == TabKind::Python)
    {
        app.sql_ui.add_tab(TabKind::Python, py.replace("\\n", "\n"));
        apply_python(app);
    }
}

// ---------------------------------------------------------------- explorer

fn explorer(app: &mut VizApp, ui: &mut Ui, t: &Tokens, actions: &mut Vec<Action>) {
    let s = &mut app.sql_ui;
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::hover());
        crate::icons::paint(ui.painter(), r, "sql", t.accent);
        ui.label(RichText::new("SQL explorer").strong().size(13.5).color(t.text));
    });
    ui.add_space(4.0);
    // Segmented control.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (k, label) in [(LeftTab::Objects, "Objects"), (LeftTab::History, "History"), (LeftTab::Snippets, "Snippets")] {
            let active = s.left == k;
            let w = (ui.available_width() / 3.0).max(60.0);
            let (r, resp) = ui.allocate_exact_size(vec2(w.min(84.0), 24.0), egui::Sense::click());
            ui.painter().rect(r, 0.0, if active { t.accent_soft } else { t.panel_alt }, egui::Stroke::new(1.0, if active { t.accent } else { t.border }), egui::StrokeKind::Inside);
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, label, FontId::proportional(12.0), if active { t.accent } else { t.text });
            if resp.clicked() {
                s.left = k;
            }
        }
    });
    ui.add_space(6.0);
    match s.left {
        LeftTab::Objects => objects(app, ui, t, actions),
        LeftTab::History => history(app, ui, t, actions),
        LeftTab::Snippets => snippets(app, ui, t, actions),
    }
}

fn objects(app: &mut VizApp, ui: &mut Ui, t: &Tokens, actions: &mut Vec<Action>) {
    let s = &mut app.sql_ui;
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut s.filter).hint_text("Filter tables and columns").desired_width(ui.available_width() - 30.0));
        if ui.add(egui::Button::new("⟳").frame(false)).on_hover_text("Refresh the object list").clicked() {
            actions.push(Action::RefreshCatalog);
        }
    });
    ui.horizontal(|ui| {
        if ui.button("+ File").on_hover_text("Attach a CSV, Parquet, JSON or Excel file as a view").clicked() {
            actions.push(Action::AttachFile);
        }
        if ui.button("+ Database").on_hover_text("Attach SQLite, PostgreSQL, MySQL or another DuckDB file").clicked() {
            s.dialog = Some(Dialog::AttachDb { kind: "sqlite".into(), target: String::new(), alias: "db".into(), error: None });
        }
    });
    ui.add_space(2.0);
    // Engine status.
    let status = match (&s.session, &s.session_error) {
        (Some(sess), _) => {
            let wh = match sess.engine.warehouse_path() {
                Some(p) => format!("Warehouse: {}", p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()),
                None => "Warehouse: in memory (save the project to keep it)".into(),
            };
            format!("DuckDB {} · {wh}", s.version)
        }
        (None, Some(e)) => e.clone(),
        (None, None) => "Starting…".into(),
    };
    ui.label(RichText::new(status).size(10.5).color(t.muted));
    ui.separator();
    let filter = s.filter.to_lowercase();
    let kinds = [ObjectKind::Model, ObjectKind::Warehouse, ObjectKind::View, ObjectKind::File, ObjectKind::External];
    egui::ScrollArea::vertical().id_salt("sql-objects").auto_shrink([false, false]).show(ui, |ui| {
        for kind in kinds {
            let objs: Vec<&CatalogObject> = s
                .catalog
                .iter()
                .filter(|o| o.kind == kind)
                .filter(|o| filter.is_empty() || o.name.to_lowercase().contains(&filter) || o.columns.iter().any(|(c, _)| c.to_lowercase().contains(&filter)))
                .collect();
            let always = matches!(kind, ObjectKind::Model | ObjectKind::Warehouse);
            if objs.is_empty() && !always {
                continue;
            }
            let head = RichText::new(format!("{}  ", kind.label().to_uppercase())).size(10.5).strong().color(t.muted);
            egui::CollapsingHeader::new(head).id_salt(("sql-kind", kind as u8)).default_open(true).show(ui, |ui| {
                if objs.is_empty() {
                    ui.label(RichText::new(match kind {
                        ObjectKind::Model => "No model tables yet: load data, then refresh.",
                        _ => "Nothing yet. CREATE TABLE … AS SELECT … saves here.",
                    }).size(11.0).italics().color(t.muted));
                }
                for o in objs {
                    object_row(ui, t, o, &filter, actions);
                }
            });
        }
    });
}

fn object_row(ui: &mut Ui, t: &Tokens, o: &CatalogObject, filter: &str, actions: &mut Vec<Action>) {
    let icon = match o.kind {
        ObjectKind::Model => "table",
        ObjectKind::Warehouse => "data",
        ObjectKind::View => "transform",
        ObjectKind::File => "open",
        ObjectKind::External => "relationship",
    };
    let id = ui.make_persistent_id(("sql-obj", &o.database, &o.schema, &o.name));
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, !filter.is_empty() && !o.name.to_lowercase().contains(filter));
    let header = state.show_header(ui, |ui| {
        let (r, _) = ui.allocate_exact_size(vec2(15.0, 15.0), egui::Sense::hover());
        crate::icons::paint(ui.painter(), r, icon, t.accent);
        let label = ui.add(egui::Label::new(RichText::new(&o.name).size(12.5).color(t.text)).sense(egui::Sense::click()));
        ui.label(RichText::new(o.columns.len().to_string()).size(10.0).color(t.muted));
        let tip = match (&o.source, o.kind) {
            (Some(src), _) => format!("{} · {src}", o.sql_name()),
            _ => format!("{}.{}.{}", o.database, o.schema, o.name),
        };
        let label = label.on_hover_text(tip);
        if label.double_clicked() {
            actions.push(Action::InsertText(o.sql_name()));
        }
        label.context_menu(|ui| object_menu(ui, o, actions));
    });
    let (_, hr, _) = header.body(|ui| {
        for (c, ty) in &o.columns {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                let ct = vizcraft_sql::col_type_of(ty);
                let badge = match ct {
                    ColType::Integer | ColType::Number => "#",
                    ColType::Date | ColType::DateTime | ColType::Time => "◷",
                    ColType::Bool => "◐",
                    ColType::Text => "Aa",
                    ColType::Any => "•",
                };
                ui.label(RichText::new(badge).size(10.5).color(grid::type_color(ct, t)));
                let hit = filter.is_empty() || c.to_lowercase().contains(filter);
                let r = ui.add(egui::Label::new(RichText::new(c).size(12.0).color(if hit { t.text } else { t.muted })).sense(egui::Sense::click()));
                if r.double_clicked() {
                    actions.push(Action::InsertText(editor::quote_if_needed(c)));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(ty.to_lowercase()).size(10.0).monospace().color(t.muted));
                });
            });
        }
    });
    let _ = hr;
}

fn object_menu(ui: &mut Ui, o: &CatalogObject, actions: &mut Vec<Action>) {
    let n = o.sql_name();
    if ui.button("SELECT TOP 100").clicked() {
        actions.push(Action::RunSqlInNewTab(format!("SELECT * FROM {n} LIMIT 100;")));
        ui.close();
    }
    if ui.button("COUNT rows").clicked() {
        actions.push(Action::RunSqlInNewTab(format!("SELECT COUNT(*) AS rows FROM {n};")));
        ui.close();
    }
    if ui.button("Describe").clicked() {
        actions.push(Action::RunSqlInNewTab(format!("DESCRIBE {n};")));
        ui.close();
    }
    if ui.button("Summarize (profile columns)").clicked() {
        actions.push(Action::RunSqlInNewTab(format!("SUMMARIZE {n};")));
        ui.close();
    }
    if ui.button("Insert name").clicked() {
        actions.push(Action::InsertText(n.clone()));
        ui.close();
    }
    ui.separator();
    if matches!(o.kind, ObjectKind::Warehouse | ObjectKind::View | ObjectKind::File | ObjectKind::External) && ui.button("Load to model").clicked() {
        actions.push(Action::LoadWarehouseTable(o.sql_name()));
        ui.close();
    }
    if ui.button("Export…").clicked() {
        actions.push(Action::Export(format!("SELECT * FROM {n}")));
        ui.close();
    }
    match o.kind {
        ObjectKind::Warehouse | ObjectKind::View => {
            ui.separator();
            if ui.button(RichText::new("Drop…").color(Color32::from_rgb(0xDC, 0x26, 0x26))).clicked() {
                actions.push(Action::Drop(o.clone()));
                ui.close();
            }
        }
        ObjectKind::File => {
            ui.separator();
            if ui.button("Detach file").clicked() {
                actions.push(Action::DetachFile(o.name.clone()));
                ui.close();
            }
        }
        _ => {}
    }
}

fn history(app: &mut VizApp, ui: &mut Ui, t: &Tokens, actions: &mut Vec<Action>) {
    let s = &mut app.sql_ui;
    let h = s.history.get_or_insert_with(store::load_history);
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} queries", h.len())).size(11.0).color(t.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("Clear").clicked() {
                actions.push(Action::ClearHistory);
            }
        });
    });
    egui::ScrollArea::vertical().id_salt("sql-history").auto_shrink([false, false]).show(ui, |ui| {
        for e in h.iter().take(300) {
            let when = chrono::DateTime::from_timestamp(e.at as i64, 0).map(|d| d.with_timezone(&chrono::Local).format("%b %d %H:%M").to_string()).unwrap_or_default();
            let meta = match (&e.error, e.rows) {
                (Some(_), _) => "error".to_string(),
                (None, Some(r)) => format!("{r} rows"),
                _ => String::new(),
            };
            let resp = egui::Frame::new().fill(t.panel_alt).corner_radius(4.0).inner_margin(egui::Margin::symmetric(8, 5)).stroke(egui::Stroke::new(1.0, t.border)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(one_line(&e.sql, 90)).monospace().size(11.0).color(t.text));
                ui.label(RichText::new(format!("{when} · {meta} · {:.0} ms", e.ms)).size(10.0).color(if e.error.is_some() { t.danger } else { t.muted }));
            });
            let r = resp.response.interact(egui::Sense::click()).on_hover_text("Click: open in a new tab · Double-click: run");
            if r.double_clicked() {
                actions.push(Action::RunSqlInNewTab(e.sql.clone()));
            } else if r.clicked() {
                actions.push(Action::OpenSql(e.sql.clone()));
            }
            ui.add_space(3.0);
        }
    });
}

fn snippets(app: &mut VizApp, ui: &mut Ui, t: &Tokens, actions: &mut Vec<Action>) {
    let s = &mut app.sql_ui;
    let cur = s.tabs.get(s.active).filter(|x| x.kind == TabKind::Sql).map(|x| x.current_statement());
    let list = s.snippets.get_or_insert_with(store::load_snippets);
    ui.horizontal(|ui| {
        if ui.button("+ Save current").on_hover_text("Save the statement under the cursor as a snippet").clicked()
            && let Some(sql) = cur.clone()
        {
            actions.push(Action::Ask(NamePurpose::SaveSnippet(sql), "My snippet".into()));
        }
    });
    egui::ScrollArea::vertical().id_salt("sql-snippets").auto_shrink([false, false]).show(ui, |ui| {
        for (i, sn) in list.iter().enumerate() {
            let resp = egui::Frame::new().fill(t.panel_alt).corner_radius(4.0).inner_margin(egui::Margin::symmetric(8, 5)).stroke(egui::Stroke::new(1.0, t.border)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(&sn.name).strong().size(12.0).color(t.text));
                ui.label(RichText::new(one_line(&sn.sql, 80)).monospace().size(10.5).color(t.muted));
            });
            let r = resp.response.interact(egui::Sense::click()).on_hover_text(&sn.sql);
            if r.clicked() {
                actions.push(Action::InsertText(sn.sql.clone()));
            }
            r.context_menu(|ui| {
                if ui.button("Insert").clicked() {
                    actions.push(Action::InsertText(sn.sql.clone()));
                    ui.close();
                }
                if ui.button("Open in new tab").clicked() {
                    actions.push(Action::OpenSql(sn.sql.clone()));
                    ui.close();
                }
                if ui.button("Delete").clicked() {
                    actions.push(Action::DeleteSnippet(i));
                    ui.close();
                }
            });
            ui.add_space(3.0);
        }
    });
}

// ---------------------------------------------------------------- tabs, toolbar, editor

fn tab_bar(app: &mut VizApp, ui: &mut Ui, t: &Tokens, actions: &mut Vec<Action>) {
    let s = &mut app.sql_ui;
    egui::Frame::new().fill(t.panel_alt).inner_margin(egui::Margin { left: 6, right: 6, top: 5, bottom: 0 }).stroke(egui::Stroke::new(1.0, t.border)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (i, tab) in s.tabs.iter().enumerate() {
                let active = i == s.active;
                let label = format!("{}{}", tab.title, if tab.running.is_some() || tab.py_run.is_some() { "  •" } else { "" });
                let w = crate::widgets::text_w(ui, &label) + 52.0;
                let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), egui::Sense::click());
                let fill = if active { t.panel } else if resp.hovered() { t.accent_soft } else { t.panel_alt };
                ui.painter().rect(r, egui::CornerRadius { nw: 5, ne: 5, sw: 0, se: 0 }, fill, egui::Stroke::new(1.0, if active { t.border } else { Color32::TRANSPARENT }), egui::StrokeKind::Inside);
                if active {
                    ui.painter().line_segment([r.left_top() + vec2(1.0, 0.5), r.right_top() + vec2(-1.0, 0.5)], egui::Stroke::new(2.0, t.accent));
                }
                let icon = if tab.kind == TabKind::Sql { "sql" } else { "python" };
                crate::icons::paint(ui.painter(), egui::Rect::from_min_size(r.min + vec2(7.0, 6.0), vec2(16.0, 16.0)), icon, if active { t.accent } else { t.muted });
                ui.painter().text(r.min + vec2(27.0, 14.0), egui::Align2::LEFT_CENTER, &label, FontId::proportional(12.0), if active { t.text } else { t.muted });
                let close = egui::Rect::from_center_size(r.right_center() - vec2(12.0, 0.0), vec2(16.0, 16.0));
                let hover_close = ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| close.contains(p));
                if resp.hovered() || active {
                    ui.painter().text(close.center(), egui::Align2::CENTER_CENTER, "✕", FontId::proportional(10.0), if hover_close { t.danger } else { t.muted });
                }
                if resp.clicked() {
                    if hover_close && s.tabs.len() > 1 {
                        actions.push(Action::CloseTab(i));
                    } else {
                        s.active = i;
                    }
                }
                if resp.middle_clicked() && s.tabs.len() > 1 {
                    actions.push(Action::CloseTab(i));
                }
            }
            ui.add_space(4.0);
            if ui.add(egui::Button::new(RichText::new("+ SQL").size(12.0)).frame(false)).on_hover_text("New SQL tab").clicked() {
                actions.push(Action::NewTab(TabKind::Sql));
            }
            if ui.add(egui::Button::new(RichText::new("+ Python").size(12.0)).frame(false)).on_hover_text("New Python tab over the last result (pandas)").clicked() {
                actions.push(Action::NewTab(TabKind::Python));
            }
        });
    });
}

fn toolbar(app: &mut VizApp, ui: &mut Ui, t: &Tokens, actions: &mut Vec<Action>) {
    let s = &mut app.sql_ui;
    let Some(tab) = s.tabs.get(s.active) else { return };
    let running = tab.running.is_some() || tab.py_run.is_some();
    let started = tab.running.map(|(_, at)| at);
    let kind = tab.kind;
    egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::symmetric(8, 5)).stroke(egui::Stroke::new(1.0, t.border)).show(ui, |ui| {
        ui.horizontal(|ui| {
            let primary = |ui: &mut Ui, label: &str, tip: &str| -> bool {
                ui.add(egui::Button::new(RichText::new(label).color(Color32::WHITE).strong()).fill(t.accent).corner_radius(4.0)).on_hover_text(tip).clicked()
            };
            match kind {
                TabKind::Sql => {
                    if primary(ui, "▶ Run", "Run the statement under the cursor, or the selection (Ctrl+Enter)") {
                        actions.push(Action::Run);
                    }
                    if ui.button("▶▶ Run all").on_hover_text("Run every statement (F5)").clicked() {
                        actions.push(Action::RunAll);
                    }
                    if ui.add_enabled(running, egui::Button::new("■ Cancel")).on_hover_text("Interrupt the running query").clicked() {
                        actions.push(Action::Cancel);
                    }
                    ui.separator();
                    if ui.button("Explain").on_hover_text("Show the query plan").clicked() {
                        actions.push(Action::Explain(false));
                    }
                    if ui.button("Explain analyze").on_hover_text("Run the query and show the plan with timings").clicked() {
                        actions.push(Action::Explain(true));
                    }
                    ui.separator();
                    if ui.button("Save snippet").clicked() {
                        actions.push(Action::Ask(NamePurpose::SaveSnippet(tab.current_statement()), "My snippet".into()));
                    }
                }
                TabKind::Python => {
                    if primary(ui, "▶ Run Python", "Run the script with the last result as `df` (Ctrl+Enter)") {
                        actions.push(Action::RunPython);
                    }
                    if ui.add_enabled(running, egui::Button::new("■ Stop")).clicked() {
                        actions.push(Action::StopPython);
                    }
                    ui.separator();
                    let df = match &s.last_result {
                        Some(r) => format!("df = {} rows × {} columns  ·  {}", r.rows.len(), r.columns.len(), one_line(&r.sql, 60)),
                        None => "df is empty: run a SQL query first".into(),
                    };
                    ui.label(RichText::new(df).size(11.5).color(t.muted));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if kind == TabKind::Sql {
                    egui::ComboBox::from_id_salt("sql-cap").selected_text(format!("{} rows", s.row_cap)).width(96.0).show_ui(ui, |ui| {
                        for n in [100usize, 1000, 10_000, 100_000, 1_000_000] {
                            ui.selectable_value(&mut s.row_cap, n, format!("{n} rows"));
                        }
                    });
                    ui.label(RichText::new("Show").size(11.5).color(t.muted));
                }
                if running {
                    ui.add(egui::Spinner::new().size(14.0));
                    if let Some(at) = started {
                        ui.label(RichText::new(format!("Running {:.1} s", crate::app::now() - at)).size(11.5).color(t.accent));
                    }
                }
            });
        });
    });
}

fn editor_area(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let SqlUi { tabs, catalog, active, .. } = &mut app.sql_ui;
    let i = (*active).min(tabs.len().saturating_sub(1));
    let Some(tab) = tabs.get_mut(i) else { return };
    let id = tab.editor_id();
    let lang = if tab.kind == TabKind::Sql { Lang::Sql } else { Lang::Python };
    let hl = if tab.kind == TabKind::Sql && tab.selection.is_none() { text::statement_at(&tab.text, tab.cursor) } else { None };
    let src_snapshot = tab.text.clone();
    let catalog: &[CatalogObject] = catalog;
    let comp = |w: &str, q: Option<&str>| completions(catalog, &src_snapshot, w, q);
    let out = editor::show(ui, t, id, &mut tab.text, lang, &comp, &mut tab.ac, hl);
    tab.cursor = out.cursor;
    tab.selection = out.selection;
}

// ---------------------------------------------------------------- results

fn results(app: &mut VizApp, ui: &mut Ui, t: &Tokens, actions: &mut Vec<Action>) {
    let last_result = app.sql_ui.last_result.clone();
    let s = &mut app.sql_ui;
    let i = s.active.min(s.tabs.len().saturating_sub(1));
    let Some(tab) = s.tabs.get_mut(i) else { return };
    if tab.kind == TabKind::Python {
        python_results(tab, last_result.as_ref(), ui, t, actions);
        return;
    }
    // Result tabs.
    egui::Frame::new().fill(t.panel_alt).inner_margin(egui::Margin::symmetric(6, 3)).show(ui, |ui| {
        ui.horizontal(|ui| {
            let mut n_res = 0;
            let mut n_plan = 0;
            for (k, o) in tab.outputs.iter().enumerate() {
                let label = match o {
                    StatementOutput::Rows(r) if !r.columns.is_empty() => {
                        n_res += 1;
                        format!("Result {n_res}  ({}{})", r.rows.len(), if r.truncated { "+" } else { "" })
                    }
                    StatementOutput::Plan { .. } => {
                        n_plan += 1;
                        format!("Plan {n_plan}")
                    }
                    _ => continue,
                };
                if ui.selectable_label(tab.result_tab == k, label).clicked() {
                    tab.result_tab = k;
                }
            }
            let errs = tab.messages.iter().filter(|m| !m.ok).count();
            let ml = if errs > 0 { RichText::new(format!("Messages ({errs} error{})", if errs == 1 { "" } else { "s" })).color(t.danger) } else { RichText::new("Messages") };
            if ui.selectable_label(tab.result_tab == usize::MAX, ml).clicked() {
                tab.result_tab = usize::MAX;
            }
            // Actions on the current result.
            if let Some(r) = tab.current_rows() {
                let sql = r.sql.clone();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Python").on_hover_text("Analyze this result in a Python tab (pandas)").clicked() {
                        actions.push(Action::OpenPython);
                    }
                    if ui.button("Save to warehouse").on_hover_text("CREATE TABLE … AS this query in the project warehouse").clicked() {
                        actions.push(Action::Ask(NamePurpose::SaveWarehouse(sql.clone()), "query_result".into()));
                    }
                    if ui.button("Load to model").on_hover_text("Add a model table built from this SQL (a Power Query query using VizCraft.Sql)").clicked() {
                        actions.push(Action::Ask(NamePurpose::LoadToModel(sql.clone()), "SQL result".into()));
                    }
                    if ui.add(egui::Button::new(RichText::new("Create chart").color(t.accent).strong())).on_hover_text("Insert a report visual over this result").clicked() {
                        actions.push(Action::Ask(NamePurpose::CreateChart(sql.clone()), "SQL chart".into()));
                    }
                    if ui.button("Export…").on_hover_text("Write the full result to CSV, Parquet or JSON").clicked() {
                        actions.push(Action::Export(sql.clone()));
                    }
                    if ui.button("Copy").on_hover_text("Copy the selection (or all rows) as tab-separated text").clicked() {
                        let gi = tab.result_tab;
                        let sel = tab.grids.get(gi).and_then(|g| g.selection());
                        let tsv = match sel {
                            Some((rr, cc)) => r.to_tsv(Some(rr), Some(cc)),
                            None => r.to_tsv(None, None),
                        };
                        ui.ctx().copy_text(tsv);
                    }
                });
            }
        });
    });
    let k = tab.result_tab;
    if tab.outputs.is_empty() && tab.messages.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new(if tab.running.is_some() { "Running…" } else { "Run a query to see results here.  Ctrl+Enter runs the statement under the cursor; F5 runs all." }).color(t.muted));
        });
        return;
    }
    if k == usize::MAX {
        egui::ScrollArea::vertical().id_salt(("sql-msgs", tab.uid)).auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(6.0);
            for m in &tab.messages {
                ui.horizontal_top(|ui| {
                    ui.add_space(8.0);
                    ui.label(RichText::new(if m.ok { "✓" } else { "✕" }).color(if m.ok { t.good } else { t.danger }).strong());
                    ui.add(egui::Label::new(RichText::new(&m.text).monospace().size(12.0).color(if m.ok { t.text } else { t.danger })).wrap());
                });
                ui.add_space(4.0);
            }
        });
        return;
    }
    let uid = tab.uid;
    if tab.grids.len() < tab.outputs.len() {
        tab.grids.resize(tab.outputs.len(), GridState::default());
    }
    let (Some(o), Some(g)) = (tab.outputs.get_mut(k), tab.grids.get_mut(k)) else { return };
    match o {
        StatementOutput::Rows(r) => {
            let footer = format!(
                "{} row{} · {} columns · {:.1} ms{}",
                r.rows.len(),
                if r.rows.len() == 1 { "" } else { "s" },
                r.columns.len(),
                r.elapsed_ms,
                if r.truncated { " · showing the first rows only: Export writes the full result" } else { "" }
            );
            egui::Panel::bottom(egui::Id::unique(("sql-foot", uid))).exact_size(22.0).frame(egui::Frame::new().fill(t.panel_alt).inner_margin(egui::Margin::symmetric(8, 3))).show(ui, |ui| {
                ui.label(RichText::new(footer).size(11.0).color(if r.truncated { t.accent } else { t.muted }));
            });
            egui::CentralPanel::default().frame(egui::Frame::new().fill(t.panel)).show(ui, |ui| grid::show(ui, t, egui::Id::unique(("sql-grid", uid, k)), r, g));
        }
        StatementOutput::Plan { text, .. } => {
            egui::ScrollArea::both().id_salt(("sql-plan", uid, k)).auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(6.0);
                ui.add(egui::Label::new(RichText::new(text.as_str()).monospace().size(12.0).color(t.text)).extend());
            });
        }
        StatementOutput::Error { message, .. } => {
            ui.label(RichText::new(message.as_str()).color(t.danger));
        }
    }
}

fn python_results(tab: &mut EditorTab, df: Option<&QueryResult>, ui: &mut Ui, t: &Tokens, actions: &mut Vec<Action>) {
    egui::Frame::new().fill(t.panel_alt).inner_margin(egui::Margin::symmetric(6, 3)).show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui.selectable_label(!tab.py_show_input, "Output").clicked() {
                tab.py_show_input = false;
            }
            if ui.selectable_label(tab.py_show_input, "Input df").clicked() {
                tab.py_show_input = true;
            }
            if let Some(o) = &tab.py_out
                && o.out.is_some()
                && !tab.py_show_input
            {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Save out to warehouse").on_hover_text("Store `out` as a warehouse table (then Load to model)").clicked() {
                        actions.push(Action::Ask(NamePurpose::SavePythonOut, "python_out".into()));
                    }
                    if ui.button("Copy").clicked()
                        && let Some(r) = &o.out
                    {
                        ui.ctx().copy_text(r.to_tsv(None, None));
                    }
                });
            }
        });
    });
    let uid = tab.uid;
    if tab.py_show_input {
        match df {
            Some(r) => {
                let mut r = r.clone();
                let mut g = GridState::default();
                grid::show(ui, t, egui::Id::unique(("py-in", uid)), &mut r, &mut g);
            }
            None => {
                ui.label(RichText::new("No input yet: run a SQL query; its result becomes `df`.").color(t.muted));
            }
        }
        return;
    }
    if tab.py_run.is_some() {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new());
            ui.label("Python is running…");
        });
        return;
    }
    let Some(o) = &mut tab.py_out else {
        ui.add_space(10.0);
        ui.label(RichText::new("Run the script: `df` holds the last SQL result as a pandas DataFrame; put a DataFrame in `out` to see it here.").color(t.muted));
        return;
    };
    let has_grid = o.out.is_some();
    let console_h = if has_grid { (ui.available_height() * 0.35).clamp(50.0, 220.0) } else { ui.available_height() };
    egui::Panel::top(egui::Id::unique(("py-console", uid))).exact_size(console_h).frame(egui::Frame::new().fill(if t.dark { Color32::from_rgb(0x12, 0x15, 0x1B) } else { Color32::from_rgb(0xFB, 0xFB, 0xFC) }).inner_margin(egui::Margin::same(8))).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt(("py-console-s", uid)).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            ui.label(RichText::new(format!("{} · {:.0} ms", if o.ok { "Finished" } else { "Failed" }, o.elapsed_ms)).size(11.0).color(if o.ok { t.good } else { t.danger }));
            if !o.stdout.is_empty() {
                ui.add(egui::Label::new(RichText::new(o.stdout.trim_end()).monospace().size(12.0).color(t.text)).extend());
            }
            if !o.stderr.is_empty() {
                ui.add(egui::Label::new(RichText::new(o.stderr.trim_end()).monospace().size(12.0).color(t.danger)).extend());
            }
        });
    });
    if let Some(r) = &mut o.out {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.panel)).show(ui, |ui| grid::show(ui, t, egui::Id::unique(("py-out", uid)), r, &mut tab.py_grid));
    }
}

// ---------------------------------------------------------------- dialogs

fn dialog(app: &mut VizApp, ctx: &egui::Context, t: &Tokens, actions: &mut Vec<Action>) {
    let Some(d) = &mut app.sql_ui.dialog else { return };
    let mut close = false;
    let mut submit: Option<Box<dyn FnOnce(&mut VizApp)>> = None;
    let mut attach: Option<(String, String, String)> = None;
    egui::Modal::new(egui::Id::unique("sql-dialog")).show(ctx, |ui| {
        ui.set_width(440.0);
        match d {
            Dialog::AttachDb { kind, target, alias, error } => {
                ui.label(RichText::new("Attach a database").strong().size(15.0));
                ui.label(RichText::new("Read tables of another database right from SQL (DuckDB extensions: they install on first use, which needs internet access once).").size(11.5).color(t.muted));
                ui.add_space(6.0);
                egui::Grid::new("attach-grid").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
                    ui.label("Kind");
                    egui::ComboBox::from_id_salt("attach-kind").selected_text(kind.as_str()).show_ui(ui, |ui| {
                        for k in ["sqlite", "postgres", "mysql", "duckdb"] {
                            ui.selectable_value(kind, k.to_string(), k);
                        }
                    });
                    ui.end_row();
                    ui.label(if kind == "sqlite" || kind == "duckdb" { "File" } else { "Connection" });
                    ui.horizontal(|ui| {
                        let hint = match kind.as_str() {
                            "postgres" => "host=localhost dbname=sales user=me",
                            "mysql" => "host=localhost database=sales user=root",
                            _ => "/path/to/database",
                        };
                        ui.add(egui::TextEdit::singleline(target).hint_text(hint).desired_width(260.0));
                        if (kind == "sqlite" || kind == "duckdb")
                            && ui.button("…").clicked()
                            && let Some(p) = rfd::FileDialog::new().pick_file()
                        {
                            *target = p.to_string_lossy().to_string();
                        }
                    });
                    ui.end_row();
                    ui.label("Name (alias)");
                    ui.add(egui::TextEdit::singleline(alias).desired_width(160.0));
                    ui.end_row();
                });
                if let Some(e) = error {
                    ui.add_space(4.0);
                    ui.label(RichText::new(e.as_str()).color(t.danger).size(11.5));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("Attach").color(Color32::WHITE)).fill(t.accent)).clicked() {
                        attach = Some((kind.clone(), target.clone(), alias.clone()));
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            }
            Dialog::Name { purpose, name } => {
                let (title, help, verb) = match purpose {
                    NamePurpose::LoadToModel(_) => ("Load to model", "A Power Query query named this runs the SQL on each refresh and loads the result as a model table.", "Load"),
                    NamePurpose::SaveWarehouse(_) => ("Save to warehouse", "CREATE OR REPLACE TABLE in the project warehouse (kept in the .duckdb file next to the project).", "Save"),
                    NamePurpose::SaveSnippet(_) => ("Save snippet", "Snippets are kept in ~/.vizcraft/sql_snippets.json.", "Save"),
                    NamePurpose::CreateChart(_) => ("Create chart", "Adds a model table from this SQL and a visual over it on the current report page.", "Create"),
                    NamePurpose::SavePythonOut => ("Save Python output", "Stores the `out` DataFrame as a warehouse table.", "Save"),
                };
                ui.label(RichText::new(title).strong().size(15.0));
                ui.label(RichText::new(help).size(11.5).color(t.muted));
                ui.add_space(6.0);
                ui.label("Name");
                let r = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                if !r.has_focus() && !r.lost_focus() {
                    r.request_focus();
                }
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if (ui.add(egui::Button::new(RichText::new(verb).color(Color32::WHITE)).fill(t.accent)).clicked() || enter) && !name.trim().is_empty() {
                        let n = name.trim().to_string();
                        let p = purpose.clone();
                        submit = Some(Box::new(move |app: &mut VizApp| name_submitted(app, p, n)));
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
    });
    let _ = actions;
    if let Some((k, tg, al)) = attach
        && let Some(s) = app.sql_ui.session.as_mut()
    {
        match s.engine.attach_database(&k, &tg, &al) {
            Ok(()) => {
                close = true;
                app.sql_ui.catalog_stale = true;
                app.sql_ui.notices.push((true, format!("Attached {al}")));
            }
            Err(e) => {
                if let Some(Dialog::AttachDb { error, .. }) = &mut app.sql_ui.dialog {
                    *error = Some(e);
                }
            }
        }
    }
    if close {
        app.sql_ui.dialog = None;
    }
    if let Some(f) = submit {
        f(app);
    }
}

fn name_submitted(app: &mut VizApp, purpose: NamePurpose, name: String) {
    match purpose {
        NamePurpose::LoadToModel(sql) => {
            let qn = app.project.queries.free_name(&name);
            app.checkpoint("Load SQL to model");
            app.project.queries.queries.push(QueryDef { name: qn.clone(), m: m_for_sql(&sql), load: LoadTo::Model, loaded_table: None, group: Some("SQL".into()), description: "Loaded from the SQL view".into() });
            app.mark_data_dirty();
            app.toast(format!("Added the model table {qn} (Power Query: VizCraft.Sql)."));
        }
        NamePurpose::SaveWarehouse(sql) => {
            let n = vizcraft_sql::quote_ident(&name);
            let inner = text::rewrite_top(sql.trim().trim_end_matches(';'));
            app.sql_ui.submit(Job::Script { sql: format!("CREATE OR REPLACE TABLE warehouse.main.{n} AS {inner}"), cap: 10 });
            app.sql_ui.catalog_stale = true;
        }
        NamePurpose::SaveSnippet(sql) => {
            let list = app.sql_ui.snippets.get_or_insert_with(store::load_snippets);
            list.insert(0, Snippet { name, sql });
            store::save_snippets(list);
            app.sql_ui.left = LeftTab::Snippets;
        }
        NamePurpose::CreateChart(sql) => create_chart(app, &sql, &name),
        NamePurpose::SavePythonOut => {
            let out = app.sql_ui.tabs.get(app.sql_ui.active).and_then(|t| t.py_out.as_ref()).and_then(|o| o.out.clone());
            if let (Some(r), Some(s)) = (out, app.sql_ui.session.as_ref()) {
                match s.engine.save_table(&name, &r.to_table_data(&name)) {
                    Ok(()) => {
                        app.sql_ui.catalog_stale = true;
                        app.toast(format!("Saved {name} to the warehouse."));
                    }
                    Err(e) => app.error(e),
                }
            }
        }
    }
}

/// Adds a model table over the SQL and a visual on the current page.
fn create_chart(app: &mut VizApp, sql: &str, name: &str) {
    let qn = app.project.queries.free_name(name);
    app.checkpoint("Create chart from SQL");
    app.project.queries.queries.push(QueryDef { name: qn.clone(), m: m_for_sql(sql), load: LoadTo::Model, loaded_table: None, group: Some("SQL".into()), description: "Created from the SQL view".into() });
    app.refresh();
    let Some(td) = app.built.table(&qn).cloned() else {
        let e = app.built.errors.iter().find(|(o, _)| *o == qn).map(|(_, e)| e.clone()).unwrap_or_default();
        app.error(format!("Couldn't load {qn}: {e}"));
        return;
    };
    let cat = (0..td.columns.len()).find(|c| !matches!(td.types.get(*c), Some(ColType::Number | ColType::Integer)));
    let nums: Vec<usize> = (0..td.columns.len()).filter(|c| matches!(td.types.get(*c), Some(ColType::Number | ColType::Integer))).collect();
    let col = |c: usize| FieldRef::col(&qn, td.columns.get(c).map(String::as_str).unwrap_or_default());
    let (kind, wells): (VisualKind, Vec<(&str, Vec<FieldUse>)>) = match (cat, nums.is_empty()) {
        (Some(c), false) => {
            let ty = td.types.get(c).copied().unwrap_or_default();
            let values: Vec<FieldUse> = nums.iter().take(3).map(|n| FieldUse::agg(col(*n), Agg::Sum)).collect();
            let kind = if ty.is_temporal() {
                VisualKind::Line
            } else if td.rows.len() > 12 {
                VisualKind::ClusteredBar
            } else {
                VisualKind::ClusteredColumn
            };
            (kind, vec![("axis", vec![FieldUse::new(col(c))]), ("values", values)])
        }
        (None, false) if td.rows.len() == 1 => (VisualKind::Card, vec![("values", nums.iter().take(1).map(|n| FieldUse::agg(col(*n), Agg::Sum)).collect())]),
        _ => (VisualKind::Table, vec![("columns", (0..td.columns.len()).map(|c| FieldUse::new(col(c))).collect())]),
    };
    let mut v = VisualSpec::new(kind);
    for (role, f) in wells {
        v.set(role, f);
    }
    let (w, h) = if kind == VisualKind::Card { (260.0, 130.0) } else { (520.0, 320.0) };
    let id = app.project.next_id();
    let (pw, ph) = app.current_page().map(|p| (p.width, p.height)).unwrap_or((1280.0, 720.0));
    let n = app.current_page().map(|p| p.elements.len()).unwrap_or(0) as f32;
    let el = Element { id, name: qn.clone(), x: (pw / 2.0 - w / 2.0 + (n % 6.0) * 16.0).max(0.0), y: (ph / 2.0 - h / 2.0 + (n % 6.0) * 16.0).max(0.0), w, h, kind: ElementKind::Visual(v), ..Element::default() };
    if app.current_page().is_none() {
        app.page = app.project.report.pages.first().map(|p| p.id).unwrap_or(0);
    }
    match app.current_page_mut() {
        Some(pg) => pg.elements.push(el),
        None => {
            app.error("There's no report page to put the chart on.");
            return;
        }
    }
    app.selected = vec![id];
    app.view = crate::app::View::Report;
    app.toast(format!("Created {} over {qn}.", kind.label()));
}

// ---------------------------------------------------------------- actions

fn apply(app: &mut VizApp, a: Action, ctx: &egui::Context) {
    let path = app.path.clone();
    let s = &mut app.sql_ui;
    match a {
        Action::Run | Action::RunAll | Action::Explain(_) => {
            if !s.ensure(path.as_deref()) {
                let e = s.session_error.clone().unwrap_or_default();
                app.error(e);
                return;
            }
            let cap = s.row_cap;
            let Some(tab) = s.tab() else { return };
            if tab.kind != TabKind::Sql || tab.running.is_some() {
                return;
            }
            let job = match a {
                Action::RunAll => Job::Script { sql: tab.text.clone(), cap },
                Action::Explain(analyze) => Job::Explain { sql: tab.current_statement(), analyze },
                _ => Job::Script { sql: tab.current_statement(), cap },
            };
            let empty = match &job {
                Job::Script { sql, .. } | Job::Explain { sql, .. } => text::split_statements(sql).is_empty(),
                Job::Export { .. } => false,
            };
            if empty {
                app.toast("Nothing to run: write a SQL statement first.");
                return;
            }
            s.submit(job);
        }
        Action::Cancel => {
            if let Some(w) = s.session.as_ref().and_then(|x| x.worker.as_ref()) {
                w.cancel();
            }
        }
        Action::RunSqlInNewTab(sql) => {
            s.add_tab(TabKind::Sql, sql);
            apply(app, Action::RunAll, ctx);
        }
        Action::OpenSql(sql) => {
            s.add_tab(TabKind::Sql, sql);
        }
        Action::NewTab(kind) => {
            let text = match kind {
                TabKind::Sql => String::new(),
                TabKind::Python => vizcraft_sql::python::TEMPLATE.to_string(),
            };
            s.add_tab(kind, text);
        }
        Action::CloseTab(i) => {
            if s.tabs.len() > 1 && i < s.tabs.len() {
                if let Some(t) = s.tabs.get_mut(i)
                    && let Some(r) = &mut t.py_run
                {
                    r.kill();
                }
                s.tabs.remove(i);
                if s.active >= i && s.active > 0 {
                    s.active -= 1;
                }
            }
        }
        Action::AttachFile => {
            if !s.ensure(path.as_deref()) {
                return;
            }
            if let Some(p) = rfd::FileDialog::new().add_filter("Data files", &["csv", "tsv", "txt", "parquet", "json", "jsonl", "ndjson", "xlsx"]).pick_file()
                && let Some(sess) = s.session.as_mut()
            {
                match sess.engine.attach_file(&p.to_string_lossy(), None) {
                    Ok(name) => {
                        s.catalog_stale = true;
                        let q = editor::quote_if_needed(&name);
                        s.add_tab(TabKind::Sql, format!("SELECT * FROM {q} LIMIT 100;"));
                        apply(app, Action::RunAll, ctx);
                    }
                    Err(e) => app.error(e),
                }
            }
        }
        Action::DetachFile(name) => {
            s.catalog_stale = true;
            if let Some(sess) = s.session.as_mut()
                && let Err(e) = sess.engine.detach_file(&name)
            {
                app.error(e);
            }
        }
        Action::Drop(o) => {
            let what = if o.kind == ObjectKind::View { "VIEW" } else { "TABLE" };
            let full = format!("{}.{}.{}", vizcraft_sql::quote_ident(&o.database), vizcraft_sql::quote_ident(&o.schema), vizcraft_sql::quote_ident(&o.name));
            s.add_tab(TabKind::Sql, format!("-- Run (Ctrl+Enter) to drop {} for good.\nDROP {what} {full};", o.name));
        }
        Action::LoadWarehouseTable(n) => {
            let name = n.trim_matches('"').rsplit('.').next().unwrap_or("table").trim_matches('"').to_string();
            name_submitted(app, NamePurpose::LoadToModel(format!("SELECT * FROM {n}")), name);
        }
        Action::Export(sql) => {
            if let Some(p) = rfd::FileDialog::new().add_filter("CSV", &["csv"]).add_filter("Parquet", &["parquet"]).add_filter("JSON", &["json"]).set_file_name("result.csv").save_file() {
                s.submit(Job::Export { sql, path: p.to_string_lossy().to_string() });
            }
        }
        Action::OpenPython => {
            if let Some(i) = s.tabs.iter().position(|t| t.kind == TabKind::Python) {
                s.active = i;
            } else {
                s.add_tab(TabKind::Python, vizcraft_sql::python::TEMPLATE.into());
            }
        }
        Action::RunPython => apply_python(app),
        Action::StopPython => {
            if let Some(t) = s.tab()
                && let Some(mut r) = t.py_run.take()
            {
                r.kill();
                t.py_out = Some(PythonOutput { stderr: "Stopped.".into(), ..PythonOutput::default() });
            }
        }
        Action::Ask(purpose, name) => s.dialog = Some(Dialog::Name { purpose, name }),
        Action::InsertText(txt) => {
            if let Some(tab) = s.tab() {
                let at = tab.cursor;
                let new = editor::insert_at(&mut tab.text, at, &txt);
                tab.cursor = new;
                editor::set_cursor(ctx, tab.editor_id(), &tab.text, new);
            }
        }
        Action::RefreshCatalog => s.catalog_stale = true,
        Action::ClearHistory => {
            store::clear_history();
            s.history = Some(vec![]);
        }
        Action::DeleteSnippet(i) => {
            if let Some(list) = &mut s.snippets
                && i < list.len()
            {
                list.remove(i);
                store::save_snippets(list);
            }
        }
    }
}

fn apply_python(app: &mut VizApp) {
    let df = app.sql_ui.last_result.clone().unwrap_or_default();
    let Some(tab) = app.sql_ui.tab() else { return };
    if tab.kind != TabKind::Python || tab.py_run.is_some() {
        return;
    }
    match vizcraft_sql::python::start(&tab.text, &df) {
        Ok(run) => {
            tab.py_run = Some(run);
            tab.py_out = None;
        }
        Err(e) => tab.py_out = Some(PythonOutput { stderr: e, ..PythonOutput::default() }),
    }
}
