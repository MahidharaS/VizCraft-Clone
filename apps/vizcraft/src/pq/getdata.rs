//! Get Data: the source dialogs (files, folders, the web, databases), the Navigator, Enter Data
//! and the "More…" gallery of connectors. Each source becomes a query with readable M
//! (`Csv.Document(File.Contents(…), [Delimiter=",", Encoding=65001, QuoteStyle=QuoteStyle.Csv])`
//! then `Table.PromoteHeaders`, `Table.TransformColumnTypes`), which loads into the model
//! (Load) or opens in the Power Query Editor (Transform Data).

use std::rc::Rc;

use egui::{Align2, FontId, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use vizcraft_query::steps::{Step, build_m, ident, steps_of};
use vizcraft_query::value::quote_text;
use vizcraft_query::{Table, Value};

use super::forms::{self, error_line, fit, index_combo, note, primary, secondary, window};
use crate::app::VizApp;
use crate::theme::Tokens;

/// An item of the Navigator (a sheet, a table, an HTML table…).
#[derive(Clone, Debug)]
pub struct NavItem {
    pub name: String,
    pub kind: String,
    pub checked: bool,
    /// The query's M when this item is loaded.
    pub m: String,
}

/// The Enter Data grid.
#[derive(Clone, Debug)]
pub struct EnterGrid {
    pub cols: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Default for EnterGrid {
    fn default() -> Self {
        EnterGrid { cols: vec!["Column1".into(), "Column2".into()], rows: vec![vec![String::new(); 2]; 3] }
    }
}

#[derive(Clone, Debug, Default)]
pub struct GetData {
    pub kind: String,
    /// File, folder, URL or server.
    pub path: String,
    pub database: String,
    pub user: String,
    pub password: String,
    pub sql: String,
    pub delimiter: usize,
    pub encoding: usize,
    pub detect: usize,
    pub no_quotes: bool,
    pub web_format: usize,
    pub headers: Vec<(String, String)>,
    pub advanced: bool,
    pub name: String,
    pub nav: Vec<NavItem>,
    pub nav_sel: Option<usize>,
    pub nav_search: String,
    /// Navigator listed for this source (path + database).
    pub nav_for: String,
    pub preview: Option<(String, Result<Rc<Table>, String>)>,
    pub enter: EnterGrid,
    pub error: Option<String>,
    /// Web: the format found when connecting.
    pub detected: String,
    steps_cache: Option<(String, Option<Vec<Step>>)>,
    typed_cache: Option<(String, String)>,
}

/// The "More…" gallery.
#[derive(Clone, Debug, Default)]
pub struct Gallery {
    pub search: String,
    pub category: usize,
    pub selected: Option<usize>,
}

const DELIMS: &[(&str, &str)] = &[("Auto", ""), ("Comma", ","), ("Semicolon", ";"), ("Tab", "#(tab)"), ("Pipe", "|"), ("Space", " "), ("Colon", ":")];
const ENCODINGS: &[(&str, u32)] = &[("65001: Unicode (UTF-8)", 65001), ("1252: Western European (Windows)", 1252), ("1200: Unicode (UTF-16)", 1200), ("28591: Western European (ISO)", 28591)];
const DETECT: &[&str] = &["Based on first 200 rows", "Based on entire dataset", "Do not detect data types"];
const WEB_FORMATS: &[&str] = &["Auto-detect", "CSV", "JSON", "HTML table", "XML", "Excel workbook", "Parquet"];

/// A connector in the gallery: (kind, name, category, description, available).
pub const CONNECTORS: &[(&str, &str, &str, &str, bool)] = &[
    ("excel", "Excel workbook", "File", "Sheets and tables of .xlsx, .xlsm, .xlsb, .xls and .ods files", true),
    ("csv", "Text/CSV", "File", "Delimited text files: CSV, TSV, pipe- or semicolon-separated", true),
    ("xml", "XML", "File", "Repeating elements of an XML document as tables", true),
    ("json", "JSON", "File", "JSON documents: arrays of objects become tables", true),
    ("parquet", "Parquet", "File", "Apache Parquet columnar files", true),
    ("folder", "Folder", "File", "Every file in a folder (and subfolders); combine them into one table", true),
    ("pdf", "PDF", "File", "Tables in PDF documents", false),
    ("sharepoint", "SharePoint folder", "File", "Files in a SharePoint document library", false),
    ("sqlserver", "SQL Server database", "Database", "Microsoft SQL Server and Azure SQL", false),
    ("postgresql", "PostgreSQL database", "Database", "PostgreSQL servers (no TLS yet)", true),
    ("mysql", "MySQL database", "Database", "MySQL and MariaDB servers (no TLS yet)", true),
    ("sqlite", "SQLite database", "Database", "SQLite database files", true),
    ("oracle", "Oracle database", "Database", "Oracle Database", false),
    ("odbc", "ODBC", "Other", "Any source with an ODBC driver", false),
    ("web", "Web", "Other", "Web pages (HTML tables), CSV, JSON or XML over HTTP(S), REST APIs", true),
    ("odata", "OData feed", "Other", "OData v4 services", true),
    ("python", "Python script", "Other", "Tables made by a Python script", true),
    ("enter", "Enter data", "Other", "Type or paste a small table", true),
    ("blank", "Blank query", "Other", "Start from an empty query in the Advanced Editor", true),
    ("sheet", "Data sheet", "Other", "A VizCraft data sheet of this project", true),
    ("salesforce", "Salesforce objects", "Online Services", "Salesforce objects and reports", false),
    ("googlesheets", "Google Sheets", "Online Services", "Sheets of a Google spreadsheet (use Web with a CSV export link)", false),
    ("github", "GitHub", "Online Services", "Use Web with the GitHub REST API", false),
];
const CATEGORIES: &[&str] = &["All", "File", "Database", "Online Services", "Other"];

pub fn kind_label(kind: &str) -> &str {
    CONNECTORS.iter().find(|c| c.0 == kind).map(|c| c.1).unwrap_or(match kind {
        "text" => "Text file",
        _ => kind,
    })
}

fn kind_icon(kind: &str) -> &'static str {
    match kind {
        "csv" | "text" => "file-csv",
        "excel" => "file-excel",
        "json" => "file-json",
        "xml" => "file-xml",
        "parquet" => "file-parquet",
        "folder" | "sharepoint" => "folder",
        "web" | "googlesheets" | "github" | "salesforce" => "web",
        "odata" => "odata",
        "sqlite" => "sqlite",
        "postgresql" => "postgresql",
        "mysql" => "mysql",
        "sqlserver" => "sqlserver",
        "oracle" => "oracle",
        "odbc" => "odbc",
        "python" => "python",
        "enter" => "enter-data",
        "blank" | "pdf" => "blank",
        "sheet" => "sheet",
        _ => "database",
    }
}

/// Opens the Get Data flow for a source kind.
pub fn open(app: &mut VizApp, kind: &str) {
    match kind {
        "more" => {
            app.pq.gallery = Some(Gallery { selected: Some(0), ..Default::default() });
        }
        "blank" => {
            let n = super::new_query(app, "Query1", "let\n    Source = \"\"\nin\n    Source".into(), true, None);
            super::open_editor(app, Some(&n));
            app.pq.dialog = Some(super::Dialog::advanced(app));
        }
        "sheet" => {
            app.pq.get_data = Some(GetData { kind: "sheet".into(), ..Default::default() });
        }
        "python" => {
            app.pq.get_data = Some(GetData { kind: "python".into(), sql: "import pandas as pd\ndata = pd.DataFrame({'Name': ['A', 'B'], 'Value': [1, 2]})\n".into(), ..Default::default() });
        }
        _ => {
            let mut gd = GetData { kind: kind.to_string(), ..Default::default() };
            if kind == "postgresql" {
                gd.path = "localhost".into();
            } else if kind == "mysql" {
                gd.path = "localhost".into();
            } else if kind == "sqlserver" {
                gd.path = "localhost".into();
            }
            // Pick a file straight away, as Power BI does.
            if matches!(kind, "csv" | "text" | "excel" | "json" | "xml" | "parquet" | "sqlite" | "folder") && std::env::var("VIZCRAFT_OPEN").is_err() {
                let d = rfd::FileDialog::new();
                let picked = match kind {
                    "folder" => d.pick_folder(),
                    "csv" | "text" => d.add_filter("Text/CSV", &["csv", "tsv", "txt", "prn", "tab"]).add_filter("All files", &["*"]).pick_file(),
                    "excel" => d.add_filter("Excel workbook", &["xlsx", "xlsm", "xlsb", "xls", "ods"]).pick_file(),
                    "json" => d.add_filter("JSON", &["json", "geojson"]).add_filter("All files", &["*"]).pick_file(),
                    "xml" => d.add_filter("XML", &["xml"]).add_filter("All files", &["*"]).pick_file(),
                    "parquet" => d.add_filter("Parquet", &["parquet"]).pick_file(),
                    _ => d.add_filter("SQLite database", &["db", "sqlite", "sqlite3", "db3"]).add_filter("All files", &["*"]).pick_file(),
                };
                match picked {
                    Some(p) => gd.path = p.to_string_lossy().to_string(),
                    None => return,
                }
            }
            if let Ok(v) = std::env::var("VIZCRAFT_OPEN_PATH") {
                gd.path = v;
            }
            app.pq.get_data = Some(gd);
        }
    }
}

/// Opens a recent source again.
pub fn open_recent(app: &mut VizApp, r: &super::Recent) {
    let gd = GetData { kind: r.kind.clone(), path: r.target.clone(), database: r.extra.clone(), ..Default::default() };
    app.pq.get_data = Some(gd);
}

fn stem(path: &str) -> String {
    std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "Query".into())
}

fn st(name: &str, expr: String) -> Step {
    Step { name: name.into(), expr }
}

/// Adds a Changed Type step typed from the data (unless detection is off).
fn with_types(app: &VizApp, mut steps: Vec<Step>, detect: bool) -> String {
    let m = build_m(&steps);
    if !detect {
        return m;
    }
    if let Ok(Value::Table(t)) = super::eval_m(app, &m) {
        let pairs = super::ops::detected_types(&t, &t.columns);
        if !pairs.is_empty() {
            let prev = steps.last().map(|s| ident(&s.name)).unwrap_or_default();
            steps.push(st("Changed Type", format!("Table.TransformColumnTypes({prev}, {{{}}})", pairs.join(", "))));
        }
    }
    build_m(&steps)
}

fn csv_options(g: &GetData) -> String {
    let mut o = Vec::new();
    if let Some((_, d)) = DELIMS.get(g.delimiter).filter(|d| !d.1.is_empty()) {
        o.push(format!("Delimiter={}", quote_text(d).replace("\"#(tab)\"", "\"#(tab)\"")));
    }
    o.push(format!("Encoding={}", ENCODINGS.get(g.encoding).map(|e| e.1).unwrap_or(65001)));
    o.push(format!("QuoteStyle=QuoteStyle.{}", if g.no_quotes { "None" } else { "Csv" }));
    format!("[{}]", o.join(", "))
}

/// The steps that read a single-table file source (before typing).
fn file_steps(g: &GetData, binary: &str) -> Vec<Step> {
    match g.kind.as_str() {
        "csv" | "text" => vec![
            st("Source", format!("Csv.Document({binary}, {})", csv_options(g))),
            st("Promoted Headers", "Table.PromoteHeaders(Source, [PromoteAllScalars=true])".into()),
        ],
        "json" => vec![st("Source", format!("Json.Document({binary})"))],
        "parquet" => vec![st("Source", format!("Parquet.Document({binary})"))],
        _ => vec![st("Source", binary.to_string())],
    }
}

/// The steps that turn a JSON document into a table, from its shape.
fn json_table_steps(app: &VizApp, mut steps: Vec<Step>) -> Vec<Step> {
    let Ok(v) = super::eval_m(app, &build_m(&steps)) else { return steps };
    let prev = steps.last().map(|s| ident(&s.name)).unwrap_or_default();
    match v {
        Value::List(_) => steps.push(st("Converted to Table", format!("Table.FromRecords({prev})"))),
        Value::Record(r) => match r.names.iter().zip(r.values.iter()).find(|(_, v)| matches!(v, Value::List(_))) {
            Some((k, _)) => {
                steps.push(st(k, format!("{prev}{}", vizcraft_query::steps::field(k))));
                steps.push(st("Converted to Table", format!("Table.FromRecords({})", ident(k))));
            }
            None => steps.push(st("Converted to Table", format!("Record.ToTable({prev})"))),
        },
        _ => {}
    }
    steps
}

/// The query steps of the dialog's current settings (single-table sources), cached.
fn source_steps(app: &VizApp, g: &mut GetData) -> Option<Vec<Step>> {
    let key = format!("{}|{}|{}|{}|{}|{}|{:?}|{}", g.kind, g.path, g.delimiter, g.encoding, g.no_quotes, g.detected, g.headers, g.sql);
    if let Some((k, v)) = &g.steps_cache
        && *k == key
    {
        return v.clone();
    }
    let mut v = raw_source_steps(g);
    let json = g.kind == "json" || (g.kind == "web" && g.detected == "json");
    if json && let Some(steps) = v.take() {
        v = Some(json_table_steps(app, steps));
    }
    g.steps_cache = Some((key, v.clone()));
    v
}

fn raw_source_steps(g: &GetData) -> Option<Vec<Step>> {
    let p = quote_text(&g.path);
    Some(match g.kind.as_str() {
        "csv" | "text" | "json" | "parquet" => file_steps(g, &format!("File.Contents({p})")),
        "odata" => vec![st("Source", format!("OData.Feed({p})"))],
        "folder" => vec![st("Source", format!("Folder.Files({p})"))],
        "web" => {
            let src = web_contents(g);
            match g.detected.as_str() {
                "csv" => vec![st("Source", format!("Csv.Document({src}, [Encoding=65001, QuoteStyle=QuoteStyle.Csv])")), st("Promoted Headers", "Table.PromoteHeaders(Source, [PromoteAllScalars=true])".into())],
                "json" => file_steps(&GetData { kind: "json".into(), ..Default::default() }, &src),
                "parquet" => vec![st("Source", format!("Parquet.Document({src})"))],
                _ => return None,
            }
        }
        "python" => vec![st("Source", format!("Python.Execute({})", quote_text(&g.sql)))],
        _ => return None,
    })
}

fn web_contents(g: &GetData) -> String {
    let headers: Vec<String> = g.headers.iter().filter(|(k, _)| !k.trim().is_empty()).map(|(k, v)| format!("{}={}", ident(k.trim()), quote_text(v))).collect();
    if headers.is_empty() { format!("Web.Contents({})", quote_text(g.path.trim())) } else { format!("Web.Contents({}, [Headers=[{}]])", quote_text(g.path.trim()), headers.join(", ")) }
}

/// Server database M: `PostgreSQL.Database("host", "db", [Query="…"])`.
fn db_call(g: &GetData, query: Option<&str>) -> String {
    let f = match g.kind.as_str() {
        "postgresql" => "PostgreSQL.Database",
        "mysql" => "MySQL.Database",
        "sqlserver" => "Sql.Database",
        _ => "Odbc.Query",
    };
    let mut opts = Vec::new();
    if !g.user.trim().is_empty() {
        opts.push(format!("Username={}", quote_text(g.user.trim())));
    }
    if !g.password.is_empty() {
        opts.push(format!("Password={}", quote_text(&g.password)));
    }
    if let Some(q) = query {
        opts.push(format!("Query={}", quote_text(q)));
    }
    let o = if opts.is_empty() { String::new() } else { format!(", [{}]", opts.join(", ")) };
    format!("{f}({}, {}{o})", quote_text(g.path.trim()), quote_text(g.database.trim()))
}

fn sqlite_call(g: &GetData, query: &str) -> String {
    format!("Sqlite.Database({}, [Query={}])", quote_text(&g.path), quote_text(query))
}

/// Lists the navigator's items for sources that have several tables.
fn list_nav(app: &VizApp, g: &mut GetData) {
    let key = format!("{}|{}|{}|{}", g.kind, g.path, g.database, g.user);
    if g.nav_for == key {
        return;
    }
    g.nav_for = key;
    g.nav.clear();
    g.nav_sel = None;
    g.error = None;
    let p = quote_text(&g.path);
    let r: Result<Vec<NavItem>, String> = match g.kind.as_str() {
        "excel" => super::eval_m(app, &format!("Excel.Workbook(File.Contents({p}), true)")).and_then(|v| match v {
            Value::Table(t) => Ok(t
                .rows
                .iter()
                .filter(|r| r.get(4).is_none_or(|h| !matches!(h, Value::Logical(true))))
                .map(|r| {
                    let name = r.first().map(Value::display).unwrap_or_default();
                    let kind = r.get(3).map(Value::display).unwrap_or_default();
                    let m = build_m(&[
                        st("Source", format!("Excel.Workbook(File.Contents({p}), true)")),
                        st(&format!("{name}_{kind}"), format!("Source{{[Item={},Kind={}]}}[Data]", quote_text(&name), quote_text(&kind))),
                    ]);
                    NavItem { name, kind, checked: false, m }
                })
                .collect()),
            _ => Err("This isn't a workbook.".into()),
        }),
        "xml" => super::eval_m(app, &format!("Xml.Tables(File.Contents({p}))")).and_then(|v| match v {
            Value::Table(t) => Ok(t
                .rows
                .iter()
                .map(|r| {
                    let name = r.first().map(Value::display).unwrap_or_default();
                    let m = build_m(&[st("Source", format!("Xml.Tables(File.Contents({p}))")), st(&name, format!("Source{{[Name={}]}}[Table]", quote_text(&name)))]);
                    NavItem { name, kind: "Table".into(), checked: false, m }
                })
                .collect()),
            _ => Err("No tables found in the XML.".into()),
        }),
        "sqlite" => vizcraft_connectors::db::sqlite_tables(&g.path).map_err(|e| e.message()).map(|v| {
            v.into_iter()
                .map(|(name, kind)| {
                    let sql = format!("SELECT * FROM {}", vizcraft_connectors::db::quote_ident(&name, false));
                    NavItem { m: build_m(&[st("Source", sqlite_call(g, &sql))]), name, kind: if kind == "view" { "View".into() } else { "Table".into() }, checked: false }
                })
                .collect()
        }),
        "postgresql" | "mysql" => {
            let list_sql = if g.kind == "mysql" {
                "SELECT table_schema, table_name, table_type FROM information_schema.tables WHERE table_schema = DATABASE() ORDER BY 1, 2"
            } else {
                "SELECT table_schema, table_name, table_type FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog','information_schema') ORDER BY 1, 2"
            };
            super::eval_m(app, &db_call(g, Some(list_sql))).and_then(|v| match v {
                Value::Table(t) => Ok(t
                    .rows
                    .iter()
                    .map(|r| {
                        let schema = r.first().map(Value::display).unwrap_or_default();
                        let name = r.get(1).map(Value::display).unwrap_or_default();
                        let bt = g.kind == "mysql";
                        let sql = format!("SELECT * FROM {}.{}", vizcraft_connectors::db::quote_ident(&schema, bt), vizcraft_connectors::db::quote_ident(&name, bt));
                        let kind = if r.get(2).map(Value::display).is_some_and(|k| k.contains("VIEW")) { "View" } else { "Table" };
                        NavItem { name: if schema == "public" || bt { name.clone() } else { format!("{schema}.{name}") }, kind: kind.into(), checked: false, m: build_m(&[st("Source", db_call(g, Some(&sql)))]) }
                    })
                    .collect()),
                _ => Err("No tables found.".into()),
            })
        }
        "web" if g.detected == "html" => super::eval_m(app, &format!("Web.Page({})", web_contents(g))).and_then(|v| match v {
            Value::Table(t) => Ok(t
                .rows
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let name = r.first().map(Value::display).unwrap_or_default();
                    let m = build_m(&[st("Source", format!("Web.Page({})", web_contents(g))), st("Data", format!("Source{{{i}}}[Data]")), st("Promoted Headers", "Table.PromoteHeaders(Data, [PromoteAllScalars=true])".into())]);
                    let m = if t.rows.get(i).and_then(|r| r.get(4)).is_some_and(|d| matches!(d, Value::Table(x) if x.columns.first().is_some_and(|c| c != "Column1"))) {
                        build_m(&[st("Source", format!("Web.Page({})", web_contents(g))), st("Data", format!("Source{{{i}}}[Data]"))])
                    } else {
                        m
                    };
                    NavItem { name, kind: "HTML table".into(), checked: false, m }
                })
                .collect()),
            _ => Err("No tables on this page.".into()),
        }),
        _ => Ok(vec![]),
    };
    match r {
        Ok(v) => {
            if v.is_empty() && matches!(g.kind.as_str(), "excel" | "xml" | "sqlite" | "postgresql" | "mysql") {
                g.error = Some("No tables were found.".into());
            }
            g.nav = v;
            let single = g.nav.len() == 1;
            if let Some(f) = g.nav.first_mut() {
                f.checked = single;
            }
            g.nav_sel = if g.nav.is_empty() { None } else { Some(0) };
        }
        Err(e) => g.error = Some(e),
    }
}

/// The M with a Changed Type step from the data (cached per source M).
fn typed_m(app: &VizApp, g: &mut GetData, steps: Vec<Step>) -> String {
    let key = format!("{}|{}", build_m(&steps), g.detect);
    if let Some((k, m)) = &g.typed_cache
        && *k == key
    {
        return m.clone();
    }
    let m = with_types(app, steps, g.detect != 2);
    g.typed_cache = Some((key, m.clone()));
    m
}

/// Evaluates M for a preview (cached by the M text). Server sources preview 200 rows.
fn preview(app: &VizApp, g: &mut GetData, m: &str) -> Result<Rc<Table>, String> {
    if let Some((k, r)) = &g.preview
        && k == m
    {
        return r.clone();
    }
    let r = super::eval_m(app, m).and_then(|v| super::as_table(v).map_err(|s| s.unwrap_or_else(|| "This source returns a single value.".into())));
    g.preview = Some((m.to_string(), r.clone()));
    r
}

/// Fetches the web source once to find its format.
fn detect_web(app: &VizApp, g: &mut GetData) {
    g.preview = None;
    g.nav_for.clear();
    if g.web_format > 0 {
        g.detected = ["", "csv", "json", "html", "xml", "excel", "parquet"].get(g.web_format).copied().unwrap_or("").to_string();
        return;
    }
    let host = vizcraft_connectors::DesktopHost;
    use vizcraft_query::Host;
    let url = {
        let mut u = g.path.trim().to_string();
        for (k, v) in &g.headers {
            if !k.trim().is_empty() {
                u.push_str(&format!("\u{1}{}: {v}", k.trim()));
            }
        }
        u
    };
    let _ = app;
    match host.web_contents(&url) {
        Ok(bytes) => {
            let head = String::from_utf8_lossy(bytes.get(..bytes.len().min(2048)).unwrap_or_default()).trim_start().to_lowercase();
            g.detected = if bytes.starts_with(b"PK") {
                "excel".into()
            } else if bytes.starts_with(b"PAR1") {
                "parquet".into()
            } else if head.starts_with('{') || head.starts_with('[') {
                "json".into()
            } else if head.starts_with("<!doctype html") || head.starts_with("<html") || head.contains("<table") {
                "html".into()
            } else if head.starts_with('<') {
                "xml".into()
            } else {
                "csv".into()
            };
            g.error = None;
        }
        Err(e) => {
            g.detected.clear();
            g.error = Some(e.message());
        }
    }
}

/// The queries to create: (name, M).
fn outputs(app: &VizApp, g: &GetData) -> Result<Vec<(String, String)>, String> {
    let detect = g.detect != 2;
    let name = if g.name.trim().is_empty() { None } else { Some(g.name.trim().to_string()) };
    match g.kind.as_str() {
        "enter" => {
            let cols: Vec<String> = g.enter.cols.iter().enumerate().map(|(i, c)| if c.trim().is_empty() { format!("Column{}", i + 1) } else { c.trim().to_string() }).collect();
            let rows: Vec<&Vec<String>> = g.enter.rows.iter().filter(|r| r.iter().any(|c| !c.trim().is_empty())).collect();
            let lits: Vec<String> = rows.iter().map(|r| format!("{{{}}}", (0..cols.len()).map(|i| cell_literal(r.get(i).map(String::as_str).unwrap_or(""))).collect::<Vec<_>>().join(", "))).collect();
            let steps = vec![st("Source", format!("#table({}, {{{}}})", vizcraft_query::steps::text_list(&cols), lits.join(", ")))];
            Ok(vec![(name.unwrap_or_else(|| "Table".into()), with_types(app, steps, true))])
        }
        "sheet" => {
            let s = g.path.clone();
            if s.is_empty() {
                return Err("Choose a data sheet.".into());
            }
            Ok(vec![(name.unwrap_or_else(|| format!("{s} Query")), build_m(&[st("Source", ident(&s))]))])
        }
        "excel" | "xml" | "sqlite" | "postgresql" | "mysql" => {
            if !g.sql.trim().is_empty() && matches!(g.kind.as_str(), "sqlite" | "postgresql" | "mysql") {
                let call = if g.kind == "sqlite" { sqlite_call(g, g.sql.trim()) } else { db_call(g, Some(g.sql.trim())) };
                return Ok(vec![(name.unwrap_or_else(|| "Query".into()), with_types(app, vec![st("Source", call)], detect && g.kind == "sqlite"))]);
            }
            let picked: Vec<&NavItem> = g.nav.iter().filter(|n| n.checked).collect();
            if picked.is_empty() {
                return Err("Tick at least one item in the Navigator.".into());
            }
            Ok(picked
                .iter()
                .map(|n| {
                    let steps = steps_of(&n.m).unwrap_or_default();
                    let m = if matches!(g.kind.as_str(), "excel" | "xml") { with_types(app, steps, detect) } else { n.m.clone() };
                    (if picked.len() == 1 { name.clone().unwrap_or_else(|| n.name.clone()) } else { n.name.clone() }, m)
                })
                .collect())
        }
        "web" if g.detected == "html" || g.detected == "excel" || g.detected == "xml" => {
            let picked: Vec<&NavItem> = g.nav.iter().filter(|n| n.checked).collect();
            if picked.is_empty() {
                return Err("Tick at least one table in the Navigator.".into());
            }
            Ok(picked.iter().map(|n| (n.name.clone(), with_types(app, steps_of(&n.m).unwrap_or_default(), detect))).collect())
        }
        "sqlserver" | "odbc" => Ok(vec![(name.unwrap_or_else(|| "Query".into()), build_m(&[st("Source", db_call(g, if g.sql.trim().is_empty() { None } else { Some(g.sql.trim()) }))]))]),
        _ => {
            let mut g2 = g.clone();
            let steps = source_steps(app, &mut g2).ok_or("Connect to the source first.")?;
            let base = match g.kind.as_str() {
                "web" | "odata" => g.path.trim_end_matches('/').rsplit('/').next().map(|s| s.split(['?', '.']).next().unwrap_or(s).to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "Web".into()),
                "python" => "Python".into(),
                _ => stem(&g.path),
            };
            Ok(vec![(name.unwrap_or(base), with_types(app, steps, detect && g.kind != "folder"))])
        }
    }
}

/// A typed cell as an M literal: numbers stay numbers, the rest is text, empty is null.
fn cell_literal(s: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        "null".into()
    } else if t.parse::<f64>().is_ok() && !(t.starts_with('0') && t.len() > 1 && !t.starts_with("0.")) {
        t.to_string()
    } else if t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("false") {
        t.to_ascii_lowercase()
    } else {
        quote_text(s)
    }
}

/// Creates the queries; `transform` opens the editor instead of loading.
pub fn finish(app: &mut VizApp, g: &GetData, transform: bool, combine: bool) -> Result<(), String> {
    let mut outs = outputs(app, g)?;
    if combine && g.kind == "folder" {
        outs = vec![(stem(&g.path), combine_m(app, g)?)];
    }
    let mut first = None;
    for (n, m) in outs {
        let q = super::new_query(app, &n, m, true, None);
        first.get_or_insert(q);
    }
    if !matches!(g.kind.as_str(), "enter" | "sheet" | "python") {
        super::remember(app, &g.kind, &g.path, &g.database);
    }
    if transform {
        super::open_editor(app, first.as_deref());
    } else {
        super::apply_to_model(app);
        app.toast(format!("Loaded {}", first.unwrap_or_default()));
    }
    Ok(())
}

/// Folder › Combine: read every file like the first one and append them.
fn combine_m(app: &VizApp, g: &GetData) -> Result<String, String> {
    let files = match super::eval_m(app, &format!("Folder.Files({})", quote_text(&g.path)))? {
        Value::Table(t) => t,
        _ => return Err("The folder couldn't be listed.".into()),
    };
    let ext_c = files.column_index("Extension").ok_or("No files")?;
    let ext = files.rows.iter().filter_map(|r| r.get(ext_c)).map(|v| v.display().to_lowercase()).find(|e| !e.is_empty()).ok_or("The folder has no files.")?;
    let mut steps = vec![
        st("Source", format!("Folder.Files({})", quote_text(&g.path))),
        st("Filtered Files", format!("Table.SelectRows(Source, each Text.Lower([Extension]) = {})", quote_text(&ext))),
        st("Transform File", format!("Table.AddColumn(#\"Filtered Files\", \"Transform File\", each {})", super::ops::file_reader(&ext))),
        st("Removed Other Columns", "Table.SelectColumns(#\"Transform File\", {\"Name\", \"Transform File\"})".into()),
    ];
    let first = match super::eval_m(app, &build_m(&steps))? {
        Value::Table(t) => t.column_index("Transform File").and_then(|c| t.rows.first().and_then(|r| r.get(c).cloned())),
        _ => None,
    };
    let Some(Value::Table(ft)) = first else { return Err("The first file couldn't be read.".into()) };
    steps.push(st("Expanded Table Column", format!("Table.ExpandTableColumn(#\"Removed Other Columns\", \"Transform File\", {})", vizcraft_query::steps::text_list(&ft.columns))));
    steps.push(st("Renamed Columns", "Table.RenameColumns(#\"Expanded Table Column\", {{\"Name\", \"Source.Name\"}})".into()));
    Ok(with_types(app, steps, true))
}

/// A compact preview grid (first rows of a table).
pub fn preview_grid(ui: &mut Ui, t: &Tokens, table: &Table, max_rows: usize, id: &str) {
    egui::ScrollArea::both().id_salt(id).auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new(id).striped(true).min_col_width(60.0).max_col_width(260.0).spacing(vec2(14.0, 3.0)).show(ui, |ui| {
            for (i, c) in table.columns.iter().enumerate() {
                let ty = table.types.get(i).copied().unwrap_or_default();
                ui.label(RichText::new(format!("{}  {c}", ty.label())).strong().size(11.5));
            }
            ui.end_row();
            for r in table.rows.iter().take(max_rows) {
                for v in r {
                    let (s, c) = match v {
                        Value::Null => ("null".to_string(), t.muted),
                        Value::Error(_) => ("Error".to_string(), t.danger),
                        Value::Table(_) => ("Table".to_string(), t.accent),
                        Value::Record(_) => ("Record".to_string(), t.accent),
                        Value::List(_) => ("List".to_string(), t.accent),
                        Value::Binary(_) => ("Binary".to_string(), t.accent),
                        v => (v.display(), t.text),
                    };
                    ui.add(egui::Label::new(RichText::new(s).size(11.5).color(c)).truncate());
                }
                ui.end_row();
            }
        });
    });
}

fn path_row(ui: &mut Ui, label: &str, path: &mut String, folder: bool, filter: Option<(&str, &[&str])>) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_sized(vec2(110.0, 22.0), egui::Label::new(label));
        changed |= ui.add(egui::TextEdit::singleline(path).desired_width((ui.available_width() - 90.0).max(200.0))).changed();
        if ui.button("Browse…").clicked() {
            let d = rfd::FileDialog::new();
            let picked = if folder {
                d.pick_folder()
            } else {
                match filter {
                    Some((n, e)) => d.add_filter(n, e).add_filter("All files", &["*"]).pick_file(),
                    None => d.pick_file(),
                }
            };
            if let Some(p) = picked {
                *path = p.to_string_lossy().to_string();
                changed = true;
            }
        }
    });
    changed
}

fn title(kind: &str) -> String {
    match kind {
        "csv" | "text" => "Text/CSV".into(),
        "enter" => "Create Table".into(),
        "sheet" => "From Data Sheet".into(),
        "python" => "Python script".into(),
        "web" => "From Web".into(),
        "odata" => "OData feed".into(),
        k => kind_label(k).to_string(),
    }
}

/// Draws the Get Data dialog and the gallery.
pub fn show(app: &mut VizApp, ctx: &egui::Context) {
    gallery(app, ctx);
    let Some(mut g) = app.pq.get_data.take() else { return };
    let t = app.tokens();
    let mut keep = true;
    let mut action: Option<(bool, bool)> = None;
    let mut open = true;
    let size = match g.kind.as_str() {
        "enter" => vec2(720.0, 480.0),
        "csv" | "text" | "json" | "parquet" | "folder" | "odata" | "web" => vec2(900.0, 600.0),
        "excel" | "xml" | "sqlite" | "postgresql" | "mysql" => vec2(940.0, 620.0),
        _ => vec2(560.0, 360.0),
    };
    let ttl = title(&g.kind);
    fit(window(&ttl), ctx, size).open(&mut open).show(ctx, |ui| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
            super::icons::paint(ui.painter(), r, kind_icon(&g.kind), t.accent, t.dark);
            ui.label(RichText::new(&ttl).size(17.0).strong());
        });
        ui.add_space(6.0);
        match g.kind.as_str() {
            "csv" | "text" | "json" | "parquet" => file_body(app, ui, &t, &mut g),
            "folder" => folder_body(app, ui, &t, &mut g),
            "excel" | "xml" => navigator_file_body(app, ui, &t, &mut g),
            "web" => web_body(app, ui, &t, &mut g),
            "odata" => odata_body(app, ui, &t, &mut g),
            "sqlite" => sqlite_body(app, ui, &t, &mut g),
            "postgresql" | "mysql" | "sqlserver" | "odbc" => server_body(app, ui, &t, &mut g),
            "enter" => enter_body(ui, &t, &mut g),
            "sheet" => {
                let sheets: Vec<String> = app.project.sheets.iter().map(|s| s.name.clone()).collect();
                if sheets.is_empty() {
                    ui.label("This project has no data sheets yet.");
                }
                ui.horizontal(|ui| {
                    ui.label("Data sheet");
                    forms::combo(ui, "gd_sheet", &mut g.path, &sheets, 260.0);
                });
            }
            "python" => {
                ui.label("Script");
                ui.label(RichText::new("Every pandas DataFrame (or list of dicts) the script leaves in a variable becomes a table.").small().color(t.muted));
                ui.add(egui::TextEdit::multiline(&mut g.sql).code_editor().desired_rows(12).desired_width(f32::INFINITY));
            }
            _ => {
                ui.label("This connector isn't available.");
            }
        }
        error_line(ui, &t, &g.error);
        ui.add_space(6.0);
        ui.separator();
        ui.horizontal(|ui| {
            if !matches!(g.kind.as_str(), "enter" | "sheet") {
                ui.label("Query name");
                ui.add(egui::TextEdit::singleline(&mut g.name).hint_text("(automatic)").desired_width(180.0));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if secondary(ui, "Cancel").clicked() {
                    keep = false;
                }
                let can = !matches!(g.kind.as_str(), "sqlserver" | "odbc");
                if g.kind == "folder" {
                    if ui.add(egui::Button::new("Transform Data").min_size(vec2(100.0, 26.0))).clicked() {
                        action = Some((true, false));
                    }
                    if ui.add(egui::Button::new("Load").min_size(vec2(70.0, 26.0))).clicked() {
                        action = Some((false, false));
                    }
                    ui.menu_button("Combine...", |ui| {
                        if ui.button("Combine & Transform Data").clicked() {
                            action = Some((true, true));
                            ui.close();
                        }
                        if ui.button("Combine & Load").clicked() {
                            action = Some((false, true));
                            ui.close();
                        }
                    });
                } else {
                    if ui.add_enabled(can, egui::Button::new("Transform Data").min_size(vec2(110.0, 26.0))).clicked() {
                        action = Some((true, false));
                    }
                    if ui.add_enabled(can, egui::Button::new(RichText::new("Load").color(egui::Color32::WHITE).strong()).fill(t.accent).min_size(vec2(84.0, 26.0))).clicked() {
                        action = Some((false, false));
                    }
                }
            });
        });
    });
    if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        keep = false;
    }
    if let Some((transform, combine)) = action {
        match finish(app, &g, transform, combine) {
            Ok(()) => keep = false,
            Err(e) => g.error = Some(e),
        }
    }
    if keep {
        app.pq.get_data = Some(g);
    }
}

fn csv_options_row(ui: &mut Ui, g: &mut GetData) -> bool {
    let before = (g.delimiter, g.encoding, g.detect, g.no_quotes);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new("File Origin").size(11.0));
            let labels: Vec<&str> = ENCODINGS.iter().map(|e| e.0).collect();
            index_combo(ui, "gd_enc", &mut g.encoding, &labels, 230.0);
        });
        ui.vertical(|ui| {
            ui.label(RichText::new("Delimiter").size(11.0));
            let labels: Vec<&str> = DELIMS.iter().map(|e| e.0).collect();
            index_combo(ui, "gd_delim", &mut g.delimiter, &labels, 130.0);
        });
        ui.vertical(|ui| {
            ui.label(RichText::new("Data Type Detection").size(11.0));
            index_combo(ui, "gd_detect", &mut g.detect, DETECT, 200.0);
        });
        ui.vertical(|ui| {
            ui.label(RichText::new("Quotes").size(11.0));
            let mut q = !g.no_quotes;
            ui.checkbox(&mut q, "Line breaks in quotes");
            g.no_quotes = !q;
        });
    });
    before != (g.delimiter, g.encoding, g.detect, g.no_quotes)
}

fn preview_box(app: &VizApp, ui: &mut Ui, t: &Tokens, g: &mut GetData, m: &str, height: f32) {
    egui::Frame::new().stroke(Stroke::new(1.0, t.border)).inner_margin(6).show(ui, |ui| {
        ui.set_min_height(height);
        ui.set_max_height(height);
        match preview(app, g, m) {
            Ok(tb) => {
                ui.label(RichText::new(format!("{} columns · {} rows (preview of the first 50)", tb.columns.len(), tb.rows.len())).size(11.0).color(t.muted));
                preview_grid(ui, t, &tb, 50, "gd_prev");
            }
            Err(e) => {
                ui.colored_label(t.danger, e);
            }
        }
    });
}

fn file_body(app: &VizApp, ui: &mut Ui, t: &Tokens, g: &mut GetData) {
    let filter: (&str, &[&str]) = match g.kind.as_str() {
        "json" => ("JSON", &["json"]),
        "parquet" => ("Parquet", &["parquet"]),
        _ => ("Text/CSV", &["csv", "tsv", "txt"]),
    };
    path_row(ui, "File path", &mut g.path, false, Some(filter));
    if matches!(g.kind.as_str(), "csv" | "text") {
        ui.add_space(4.0);
        csv_options_row(ui, g);
    } else {
        ui.horizontal(|ui| {
            ui.label("Data Type Detection");
            index_combo(ui, "gd_detect", &mut g.detect, DETECT, 200.0);
        });
    }
    ui.add_space(6.0);
    if !std::path::Path::new(&g.path).is_file() {
        note(ui, t, "Choose a file to see a preview.", false);
        return;
    }
    let Some(steps) = source_steps(app, g) else { return };
    let m = typed_m(app, g, steps);
    let h = (ui.available_height() - 60.0).max(200.0);
    preview_box(app, ui, t, g, &m, h);
}

fn folder_body(app: &VizApp, ui: &mut Ui, t: &Tokens, g: &mut GetData) {
    path_row(ui, "Folder path", &mut g.path, true, None);
    ui.label(RichText::new("Lists every file in the folder and its subfolders. Combine reads them all the same way (like the first file) and appends them.").size(11.0).color(t.muted));
    ui.add_space(6.0);
    if !std::path::Path::new(&g.path).is_dir() {
        note(ui, t, "Choose a folder to see its files.", false);
        return;
    }
    let m = format!("Table.RemoveColumns(Folder.Files({}), {{\"Content\"}})", quote_text(&g.path));
    let h = (ui.available_height() - 60.0).max(200.0);
    preview_box(app, ui, t, g, &m, h);
}

/// The Navigator: items with checkboxes on the left, the selected item's preview on the right.
fn navigator(app: &VizApp, ui: &mut Ui, t: &Tokens, g: &mut GetData) {
    let h = (ui.available_height() - 64.0).max(260.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(260.0);
            ui.add(egui::TextEdit::singleline(&mut g.nav_search).hint_text("Search").desired_width(250.0));
            let n_checked = g.nav.iter().filter(|n| n.checked).count();
            ui.label(RichText::new(format!("Display options · {n_checked} selected")).size(11.0).color(t.muted));
            egui::Frame::new().stroke(Stroke::new(1.0, t.border)).inner_margin(4).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("gd_nav").max_height(h - 50.0).min_scrolled_height(h - 50.0).show(ui, |ui| {
                    ui.set_min_width(240.0);
                    let q = g.nav_search.to_lowercase();
                    let none_checked = !g.nav.iter().any(|n| n.checked);
                    let mut tick: Option<usize> = None;
                    for (i, n) in g.nav.iter_mut().enumerate() {
                        if !q.is_empty() && !n.name.to_lowercase().contains(&q) {
                            continue;
                        }
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut n.checked, "");
                            let icon = match n.kind.as_str() {
                                "Sheet" => "sheet",
                                "View" => "query",
                                _ => "query",
                            };
                            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                            super::icons::paint(ui.painter(), r, icon, t.accent, t.dark);
                            let sel = g.nav_sel == Some(i);
                            if ui.selectable_label(sel, &n.name).on_hover_text(&n.kind).clicked() {
                                g.nav_sel = Some(i);
                                if none_checked {
                                    tick = Some(i);
                                }
                            }
                        });
                    }
                    if let Some(n) = tick.and_then(|i| g.nav.get_mut(i)) {
                        n.checked = true;
                    }
                });
            });
        });
        ui.vertical(|ui| {
            let m = g.nav_sel.and_then(|i| g.nav.get(i)).map(|n| (n.name.clone(), n.m.clone()));
            match m {
                Some((name, m)) => {
                    ui.label(RichText::new(name).strong().size(13.0));
                    // Server sources preview a limited query.
                    let pm = if matches!(g.kind.as_str(), "sqlite" | "postgresql" | "mysql") { limit_query(&m) } else { m };
                    preview_box(app, ui, t, g, &pm, h - 24.0);
                }
                None => {
                    ui.label(RichText::new("Select an item to preview it, tick the ones to load.").color(t.muted));
                }
            }
        });
    });
}

/// Adds `LIMIT 200` to a navigator item's native query (for the preview).
fn limit_query(m: &str) -> String {
    match m.rfind("SELECT * FROM ") {
        Some(_) => {
            // The query text sits in a quoted M string: append before its closing quote.
            let Some(q) = m.find("Query=\"") else { return m.to_string() };
            let start = q + 7;
            let mut i = start;
            let b = m.as_bytes();
            while i < b.len() {
                if b.get(i) == Some(&b'"') {
                    if b.get(i + 1) == Some(&b'"') {
                        i += 2;
                        continue;
                    }
                    break;
                }
                i += 1;
            }
            format!("{} LIMIT 200{}", m.get(..i).unwrap_or_default(), m.get(i..).unwrap_or_default())
        }
        None => m.to_string(),
    }
}

fn navigator_file_body(app: &VizApp, ui: &mut Ui, t: &Tokens, g: &mut GetData) {
    let filter: (&str, &[&str]) = if g.kind == "excel" { ("Excel workbook", &["xlsx", "xlsm", "xlsb", "xls", "ods"]) } else { ("XML", &["xml"]) };
    path_row(ui, "File path", &mut g.path, false, Some(filter));
    ui.horizontal(|ui| {
        ui.label("Data Type Detection");
        index_combo(ui, "gd_detect", &mut g.detect, DETECT, 200.0);
    });
    ui.add_space(4.0);
    if !std::path::Path::new(&g.path).is_file() {
        note(ui, t, "Choose a file: its sheets and tables appear in the Navigator.", false);
        return;
    }
    list_nav(app, g);
    navigator(app, ui, t, g);
}

fn web_body(app: &VizApp, ui: &mut Ui, t: &Tokens, g: &mut GetData) {
    ui.horizontal(|ui| {
        ui.radio_value(&mut g.advanced, false, "Basic");
        ui.radio_value(&mut g.advanced, true, "Advanced");
    });
    let mut connect = false;
    ui.horizontal(|ui| {
        ui.add_sized(vec2(60.0, 22.0), egui::Label::new("URL"));
        let r = ui.add(egui::TextEdit::singleline(&mut g.path).hint_text("https://example.com/data.csv").desired_width((ui.available_width() - 100.0).max(200.0)));
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            connect = true;
        }
        if primary(ui, t, "Connect").clicked() {
            connect = true;
        }
    });
    if g.advanced {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Format");
            index_combo(ui, "gd_webfmt", &mut g.web_format, WEB_FORMATS, 160.0);
        });
        ui.label(RichText::new("HTTP request header parameters (optional)").size(11.0).color(t.muted));
        let mut remove = None;
        for (i, (k, v)) in g.headers.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(k).hint_text("Header").desired_width(160.0));
                ui.add(egui::TextEdit::singleline(v).hint_text("Value").desired_width(300.0));
                if forms::glyph_button(ui, t, 'x', t.muted, "Remove this header").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            g.headers.remove(i);
        }
        if ui.small_button("Add header").clicked() {
            g.headers.push(("Accept".into(), "application/json".into()));
        }
        if g.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("authorization")) {
            note(ui, t, "Headers are saved in the query (and in the project file). Prefer a parameter or an environment-specific token.", true);
        }
    }
    if connect && !g.path.trim().is_empty() {
        detect_web(app, g);
    }
    ui.add_space(6.0);
    if g.detected.is_empty() {
        note(ui, t, "Enter a URL and Connect. CSV, JSON, XML, Excel, Parquet and HTML pages (their tables) are detected automatically. HTTPS_PROXY is respected.", false);
        return;
    }
    ui.label(RichText::new(format!("Detected format: {}", g.detected.to_uppercase())).size(11.0).color(t.muted));
    match g.detected.as_str() {
        "html" => {
            list_nav(app, g);
            navigator(app, ui, t, g);
        }
        "excel" | "xml" => {
            let key = format!("web-{}{}", g.detected, g.path);
            if g.nav_for != key {
                g.nav_for = key;
                let src = web_contents(g);
                let list = if g.detected == "excel" { format!("Excel.Workbook({src}, true)") } else { format!("Xml.Tables({src})") };
                g.nav = match super::eval_m(app, &list) {
                    Ok(Value::Table(tb)) => tb
                        .rows
                        .iter()
                        .map(|r| {
                            let name = r.first().map(Value::display).unwrap_or_default();
                            let pick = if g.detected == "excel" { format!("Source{{[Name={}]}}[Data]", quote_text(&name)) } else { format!("Source{{[Name={}]}}[Table]", quote_text(&name)) };
                            NavItem { m: build_m(&[st("Source", list.clone()), st(&name, pick)]), name, kind: "Table".into(), checked: false }
                        })
                        .collect(),
                    Ok(_) => vec![],
                    Err(e) => {
                        g.error = Some(e);
                        vec![]
                    }
                };
                g.nav_sel = if g.nav.is_empty() { None } else { Some(0) };
            }
            navigator(app, ui, t, g);
        }
        _ => {
            if let Some(steps) = source_steps(app, g) {
                let m = typed_m(app, g, steps);
                let h = (ui.available_height() - 60.0).max(200.0);
                preview_box(app, ui, t, g, &m, h);
            }
        }
    }
}

fn odata_body(app: &VizApp, ui: &mut Ui, t: &Tokens, g: &mut GetData) {
    let mut connect = false;
    ui.horizontal(|ui| {
        ui.add_sized(vec2(60.0, 22.0), egui::Label::new("URL"));
        ui.add(egui::TextEdit::singleline(&mut g.path).hint_text("https://services.odata.org/V4/Northwind/Northwind.svc/Products").desired_width((ui.available_width() - 100.0).max(200.0)));
        connect = primary(ui, t, "Connect").clicked();
    });
    if connect {
        g.detected = "odata".into();
        g.preview = None;
    }
    ui.add_space(6.0);
    if g.detected != "odata" {
        note(ui, t, "Enter an OData feed URL (an entity set) and Connect.", false);
        return;
    }
    let m = build_m(&[st("Source", format!("OData.Feed({})", quote_text(&g.path)))]);
    let h = (ui.available_height() - 60.0).max(200.0);
    preview_box(app, ui, t, g, &m, h);
}

fn sqlite_body(app: &VizApp, ui: &mut Ui, t: &Tokens, g: &mut GetData) {
    path_row(ui, "Database file", &mut g.path, false, Some(("SQLite database", &["db", "sqlite", "sqlite3", "db3"])));
    egui::CollapsingHeader::new("Advanced options").id_salt("gd_sqlite_adv").show(ui, |ui| {
        ui.label("SQL statement (optional)");
        ui.add(egui::TextEdit::multiline(&mut g.sql).code_editor().desired_rows(3).desired_width(f32::INFINITY).hint_text("SELECT region, SUM(amount) FROM orders GROUP BY region"));
    });
    ui.add_space(4.0);
    if !std::path::Path::new(&g.path).is_file() {
        note(ui, t, "Choose a SQLite database file: its tables and views appear in the Navigator.", false);
        return;
    }
    if !g.sql.trim().is_empty() {
        let m = build_m(&[st("Source", sqlite_call(g, g.sql.trim()))]);
        let h = (ui.available_height() - 60.0).max(200.0);
        preview_box(app, ui, t, g, &m, h);
        return;
    }
    list_nav(app, g);
    navigator(app, ui, t, g);
}

fn server_body(app: &VizApp, ui: &mut Ui, t: &Tokens, g: &mut GetData) {
    let available = matches!(g.kind.as_str(), "postgresql" | "mysql");
    if !available {
        note(
            ui,
            t,
            if g.kind == "sqlserver" {
                "The SQL Server connector isn't available in this build yet (its driver needs an async runtime). Export the data to CSV or Parquet, or use PostgreSQL, MySQL or SQLite."
            } else {
                "ODBC isn't available in this build yet: it needs the system's ODBC driver manager."
            },
            true,
        );
        ui.add_space(6.0);
    }
    let mut connect = false;
    egui::Grid::new("gd_server").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.label("Server");
        ui.add(egui::TextEdit::singleline(&mut g.path).hint_text("host or host:port").desired_width(320.0));
        ui.end_row();
        ui.label("Database");
        ui.add(egui::TextEdit::singleline(&mut g.database).desired_width(320.0));
        ui.end_row();
        ui.label("Data Connectivity mode");
        ui.label(RichText::new("Import").color(t.muted));
        ui.end_row();
        ui.label("User name");
        ui.add(egui::TextEdit::singleline(&mut g.user).hint_text(if g.kind == "mysql" { "(MYSQL_USER or root)" } else { "(PGUSER or postgres)" }).desired_width(220.0));
        ui.end_row();
        ui.label("Password");
        ui.add(egui::TextEdit::singleline(&mut g.password).password(true).hint_text(if g.kind == "mysql" { "(MYSQL_PASSWORD)" } else { "(PGPASSWORD)" }).desired_width(220.0));
        ui.end_row();
    });
    if !g.password.is_empty() {
        note(ui, t, "The user name and password are saved in the query's M, in the project file. Leave them empty to use the environment variables instead.", true);
    }
    egui::CollapsingHeader::new("Advanced options").id_salt("gd_srv_adv").show(ui, |ui| {
        ui.label("SQL statement (optional; requires a database)");
        ui.add(egui::TextEdit::multiline(&mut g.sql).code_editor().desired_rows(3).desired_width(f32::INFINITY));
    });
    ui.horizontal(|ui| {
        if available && primary(ui, t, "Connect").clicked() {
            connect = true;
        }
        ui.label(RichText::new("Connections are unencrypted (no TLS) in this build.").size(11.0).color(t.muted));
    });
    if !available {
        return;
    }
    if connect {
        g.nav_for.clear();
        g.preview = None;
        if g.sql.trim().is_empty() {
            list_nav(app, g);
        } else {
            g.detected = "sql".into();
        }
    }
    ui.add_space(4.0);
    if !g.sql.trim().is_empty() && g.detected == "sql" {
        let m = build_m(&[st("Source", db_call(g, Some(g.sql.trim())))]);
        let h = (ui.available_height() - 60.0).max(160.0);
        preview_box(app, ui, t, g, &m, h);
    } else if !g.nav.is_empty() {
        navigator(app, ui, t, g);
    }
}

fn enter_body(ui: &mut Ui, t: &Tokens, g: &mut GetData) {
    ui.label(RichText::new("Type or paste data (Ctrl+V of cells from a spreadsheet fills the grid). Double-click a header to rename the column.").size(11.5).color(t.muted));
    ui.add_space(4.0);
    let grid = &mut g.enter;
    let mut paste: Option<(usize, usize, String)> = None;
    let h = (ui.available_height() - 90.0).max(200.0);
    egui::Frame::new().stroke(Stroke::new(1.0, t.border)).show(ui, |ui| {
        egui::ScrollArea::both().id_salt("gd_enter").max_height(h).min_scrolled_height(h).auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("gd_enter_grid").spacing(vec2(0.0, 0.0)).show(ui, |ui| {
                ui.label("");
                let ncols = grid.cols.len();
                for c in 0..ncols {
                    if let Some(name) = grid.cols.get_mut(c) {
                        egui::Frame::new().fill(t.panel_alt).stroke(Stroke::new(0.5, t.border)).show(ui, |ui| {
                            ui.add_sized(vec2(130.0, 22.0), egui::TextEdit::singleline(name).font(FontId::proportional(12.0)).frame(egui::Frame::NONE).text_color(t.text).margin(egui::Margin::symmetric(5, 3)));
                        });
                    }
                }
                if ui.add(egui::Button::new("+").min_size(vec2(26.0, 22.0))).on_hover_text("Add a column").clicked() {
                    grid.cols.push(format!("Column{}", grid.cols.len() + 1));
                    for r in grid.rows.iter_mut() {
                        r.push(String::new());
                    }
                }
                ui.end_row();
                let nrows = grid.rows.len();
                for r in 0..nrows {
                    ui.label(RichText::new((r + 1).to_string()).size(11.0).color(t.muted));
                    for c in 0..ncols {
                        if let Some(cell) = grid.rows.get_mut(r).and_then(|row| row.get_mut(c)) {
                            let resp = egui::Frame::new().stroke(Stroke::new(0.5, t.border)).show(ui, |ui| ui.add_sized(vec2(130.0, 22.0), egui::TextEdit::singleline(cell).frame(egui::Frame::NONE).margin(egui::Margin::symmetric(5, 3)))).inner;
                            if resp.changed() && (cell.contains('\t') || cell.contains('\n')) {
                                paste = Some((r, c, std::mem::take(cell)));
                            }
                        }
                    }
                    ui.end_row();
                }
                ui.label("");
                if ui.add(egui::Button::new("+").min_size(vec2(26.0, 22.0))).on_hover_text("Add a row").clicked() {
                    grid.rows.push(vec![String::new(); grid.cols.len()]);
                }
                ui.end_row();
            });
        });
    });
    if let Some((r0, c0, text)) = paste {
        let lines: Vec<&str> = text.trim_end_matches(['\n', '\r']).lines().collect();
        for (i, line) in lines.iter().enumerate() {
            for (j, v) in line.split('\t').enumerate() {
                let (r, c) = (r0 + i, c0 + j);
                while grid.cols.len() <= c {
                    let n = grid.cols.len() + 1;
                    grid.cols.push(format!("Column{n}"));
                    for row in grid.rows.iter_mut() {
                        row.push(String::new());
                    }
                }
                while grid.rows.len() <= r {
                    grid.rows.push(vec![String::new(); grid.cols.len()]);
                }
                if let Some(cell) = grid.rows.get_mut(r).and_then(|row| row.get_mut(c)) {
                    *cell = v.trim_end_matches('\r').to_string();
                }
            }
        }
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label("Name");
        ui.add(egui::TextEdit::singleline(&mut g.name).hint_text("Table").desired_width(200.0));
        if ui.button("Use first row as headers").clicked() && !g.enter.rows.is_empty() {
            let first = g.enter.rows.remove(0);
            for (i, c) in g.enter.cols.iter_mut().enumerate() {
                if let Some(v) = first.get(i).filter(|v| !v.trim().is_empty()) {
                    *c = v.clone();
                }
            }
        }
    });
}

fn gallery(app: &mut VizApp, ctx: &egui::Context) {
    let Some(mut gal) = app.pq.gallery.take() else { return };
    let t = app.tokens();
    let mut keep = true;
    let mut connect: Option<&str> = None;
    let mut open = true;
    fit(window("Get Data"), ctx, vec2(820.0, 560.0)).open(&mut open).show(ctx, |ui| {
        ui.add(egui::TextEdit::singleline(&mut gal.search).hint_text("Search").desired_width(260.0));
        ui.add_space(6.0);
        let h = 420.0;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(150.0);
                for (i, c) in CATEGORIES.iter().enumerate() {
                    let r = ui.add(egui::Button::selectable(gal.category == i, *c).min_size(vec2(146.0, 26.0)));
                    if r.clicked() {
                        gal.category = i;
                        gal.selected = None;
                    }
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.set_width(330.0);
                let cat = CATEGORIES.get(gal.category).copied().unwrap_or("All");
                ui.label(RichText::new(cat).strong().size(13.0));
                egui::ScrollArea::vertical().id_salt("gd_gallery").max_height(h).min_scrolled_height(h).show(ui, |ui| {
                    let q = gal.search.to_lowercase();
                    for (i, c) in CONNECTORS.iter().enumerate() {
                        if (cat != "All" && c.2 != cat) || (!q.is_empty() && !c.1.to_lowercase().contains(&q) && !c.3.to_lowercase().contains(&q)) {
                            continue;
                        }
                        let sel = gal.selected == Some(i);
                        let (rect, resp) = ui.allocate_exact_size(vec2(320.0, 30.0), Sense::click());
                        if sel {
                            ui.painter().rect_filled(rect, 4.0, t.accent_soft);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(rect, 4.0, t.panel_alt);
                        }
                        super::icons::paint(ui.painter(), Rect::from_min_size(rect.min + vec2(6.0, 5.0), vec2(20.0, 20.0)), kind_icon(c.0), if c.4 { t.accent } else { t.muted }, t.dark);
                        ui.painter().text(rect.min + vec2(34.0, 15.0), Align2::LEFT_CENTER, c.1, FontId::proportional(12.5), if c.4 { t.text } else { t.muted });
                        if !c.4 {
                            ui.painter().text(pos2(rect.max.x - 8.0, rect.center().y), Align2::RIGHT_CENTER, "Coming soon", FontId::proportional(10.5), t.muted);
                        }
                        if resp.clicked() {
                            gal.selected = Some(i);
                        }
                        if resp.double_clicked() && c.4 {
                            connect = Some(c.0);
                        }
                    }
                });
            });
            ui.separator();
            ui.vertical(|ui| {
                match gal.selected.and_then(|i| CONNECTORS.get(i)) {
                    Some(c) => {
                        let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
                        super::icons::paint(ui.painter(), r, kind_icon(c.0), t.accent, t.dark);
                        ui.label(RichText::new(c.1).strong().size(15.0));
                        ui.label(RichText::new(c.2).size(11.0).color(t.muted));
                        ui.add_space(6.0);
                        ui.label(c.3);
                        if !c.4 {
                            ui.add_space(6.0);
                            note(ui, &t, "Not available in this build yet.", true);
                        }
                    }
                    None => {
                        ui.label(RichText::new("Select a connector").color(t.muted));
                    }
                }
            });
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} connectors", CONNECTORS.len())).size(11.0).color(t.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if secondary(ui, "Cancel").clicked() {
                    keep = false;
                }
                let can = gal.selected.and_then(|i| CONNECTORS.get(i)).is_some_and(|c| c.4);
                if ui.add_enabled(can, egui::Button::new(RichText::new("Connect").color(egui::Color32::WHITE).strong()).fill(t.accent).min_size(vec2(84.0, 26.0))).clicked() {
                    connect = gal.selected.and_then(|i| CONNECTORS.get(i)).map(|c| c.0);
                }
            });
        });
    });
    if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        keep = false;
    }
    if let Some(k) = connect {
        keep = false;
        open_kind(app, k);
    }
    if keep {
        app.pq.gallery = Some(gal);
    }
}

fn open_kind(app: &mut VizApp, k: &str) {
    open(app, k);
}
