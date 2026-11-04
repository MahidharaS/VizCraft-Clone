//! The **VizCraft Project folder format** (`.vizp`): a Git-friendly, agent-friendly way to store a
//! project as many small text files, like Power BI's PBIP/PBIR.
//!
//! A project saved as `Sales.vizp` is a small pointer file plus sibling folders:
//!
//! ```text
//! Sales.vizp                       pointer: name, format version, the folders below
//! Sales.SemanticModel/
//!   model.json                     culture, auto date/time, relationships, roles, parameters, layouts
//!   tables/<Table>.json            one file per model table (columns, hierarchies, source)
//!   measures/<Table>/<Name>.dax    one DAX expression per measure; metadata in measures.json
//!   columns/<Table>/<Name>.dax     calculated columns; metadata in columns.json
//!   queries/<Query>.m              one Power Query M script per query; metadata in queries.json
//!   parameters.json                query parameters
//! Sales.Report/
//!   report.json                    theme, settings, report filters, bookmarks, stories, page order
//!   pages/page-<id>/page.json      page name, settings and filters, visual order
//!   pages/page-<id>/visuals/<id>.json   one file per visual / shape / text box
//! Sales.Data/
//!   sheets/<Sheet>.csv             data sheet cells (row 1 = headers); metadata in <Sheet>.json
//!   flows/<Flow>.json              Fabric-style pipelines
//!   notebooks/<Notebook>.json      notebooks
//! Sales.Science/
//!   workflows/<Workflow>.json      Data Science Studio workflows
//!   models/<name>-v<N>.json        registered model versions
//! Sales.Settings/
//!   publish.json                   publishing targets
//! ```
//!
//! Everything round-trips: [`load`] rebuilds exactly the [`Project`] that [`save`] wrote. Files
//! are pretty JSON with stable key order, so `git diff` shows real changes, and an agent (or a
//! person in VS Code) can edit a measure, a query or a visual by editing one small file. Files no
//! longer part of the project are removed on save; files outside these folders are never touched.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::doc::Project;

/// File extension of the pointer file.
pub const EXT: &str = "vizp";
/// Format version written to the pointer file.
pub const FOLDER_VERSION: &str = "1.0";

const PARTS: [&str; 5] = ["SemanticModel", "Report", "Data", "Science", "Settings"];

fn err(m: impl Into<String>) -> crate::Error {
    crate::Error::Msg(m.into())
}

/// A file-name-safe version of `s` (keeps letters, digits, spaces, `-_.()`; max 80 chars).
pub fn safe_name(s: &str) -> String {
    let mut out: String = s.chars().map(|c| if c.is_alphanumeric() || " -_.()".contains(c) { c } else { '_' }).collect();
    out = out.trim().trim_matches('.').to_string();
    if out.is_empty() {
        out = "_".into();
    }
    out.chars().take(80).collect()
}

/// Unique file stems for a list of names (adds ` (2)`, ` (3)`… on clashes, case-insensitively).
fn unique(names: &[String]) -> Vec<String> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    names
        .iter()
        .map(|n| {
            let base = safe_name(n);
            let k = base.to_lowercase();
            let c = seen.entry(k).or_insert(0);
            *c += 1;
            if *c == 1 { base } else { format!("{base} ({c})") }
        })
        .collect()
}

/// The folder for one part (`Sales.Report`) next to the pointer file.
pub fn part_dir(pointer: &Path, part: &str) -> PathBuf {
    let stem = pointer.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Project".into());
    pointer.with_file_name(format!("{stem}.{part}"))
}

/// Files to write: relative path → content.
type Files = BTreeMap<PathBuf, String>;

fn pretty(v: &Value) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap_or_default();
    s.push('\n');
    s
}

fn take(m: &mut Map<String, Value>, k: &str) -> Value {
    m.remove(k).unwrap_or(Value::Null)
}

fn arr(v: Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a,
        _ => vec![],
    }
}

fn s_of(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) || s.starts_with(' ') || s.ends_with(' ') { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

fn to_csv(rows: &[Value]) -> String {
    let mut out = String::new();
    for r in rows {
        let cells: Vec<String> = r.as_array().map(|c| c.iter().map(|v| csv_escape(v.as_str().unwrap_or(""))).collect()).unwrap_or_default();
        out.push_str(&cells.join(","));
        out.push('\n');
    }
    out
}

/// RFC-4180 CSV parsing (quoted fields, doubled quotes, newlines inside quotes).
pub fn parse_csv(s: &str) -> Vec<Vec<String>> {
    let mut rows = vec![];
    let mut row = vec![];
    let mut cell = String::new();
    let mut q = false;
    let mut chars = s.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        if q {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cell.push('"');
                } else {
                    q = false;
                }
            } else {
                cell.push(c);
            }
        } else {
            match c {
                '"' => q = true,
                ',' => row.push(std::mem::take(&mut cell)),
                '\r' => {}
                '\n' => {
                    row.push(std::mem::take(&mut cell));
                    rows.push(std::mem::take(&mut row));
                    any = false;
                }
                _ => cell.push(c),
            }
        }
    }
    if any {
        row.push(cell);
        rows.push(row);
    }
    rows
}

/// Splits a project into files (relative to the pointer's directory).
pub fn to_files(p: &Project, stem: &str) -> Files {
    let mut files = Files::new();
    let Value::Object(mut root) = serde_json::to_value(p).unwrap_or(Value::Null) else { return files };
    let dir = |part: &str| PathBuf::from(format!("{stem}.{part}"));

    // ---------------------------------------------------------------- semantic model
    let sm = dir("SemanticModel");
    let mut model = match take(&mut root, "model") {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    let tables = arr(take(&mut model, "tables"));
    let tnames = unique(&tables.iter().map(|t| s_of(t, "name")).collect::<Vec<_>>());
    for (t, n) in tables.iter().zip(&tnames) {
        files.insert(sm.join("tables").join(format!("{n}.json")), pretty(t));
    }
    model.insert("tableOrder".into(), json!(tnames));
    for (key, sub, ext) in [("measures", "measures", "dax"), ("calculatedColumns", "columns", "dax")] {
        let items = arr(take(&mut model, key));
        let mut meta = vec![];
        let mut by_table: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for it in &items {
            by_table.entry(s_of(it, "table")).or_default().push(s_of(it, "name"));
        }
        let mut stems: BTreeMap<String, std::collections::VecDeque<String>> = by_table.iter().map(|(t, ns)| (t.clone(), unique(ns).into_iter().collect())).collect();
        for it in items {
            let Value::Object(mut m) = it else { continue };
            let table = m.get("table").and_then(Value::as_str).unwrap_or("").to_string();
            let file_stem = stems.get_mut(&table).and_then(|q| q.pop_front()).unwrap_or_else(|| "_".into());
            let expr = m.remove("expression").and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
            let rel = PathBuf::from(sub).join(safe_name(if table.is_empty() { "_" } else { &table })).join(format!("{file_stem}.{ext}"));
            files.insert(sm.join(&rel), format!("{}\n", expr.trim_end()));
            m.insert("file".into(), json!(rel.to_string_lossy().replace('\\', "/")));
            meta.push(Value::Object(m));
        }
        files.insert(sm.join(format!("{sub}.json")), pretty(&Value::Array(meta)));
    }
    files.insert(sm.join("model.json"), pretty(&Value::Object(model)));

    let mut qdoc = match take(&mut root, "queries") {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    let queries = arr(take(&mut qdoc, "queries"));
    let qnames = unique(&queries.iter().map(|q| s_of(q, "name")).collect::<Vec<_>>());
    let mut qmeta = vec![];
    for (q, n) in queries.into_iter().zip(&qnames) {
        let Value::Object(mut m) = q else { continue };
        let code = m.remove("m").and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
        files.insert(sm.join("queries").join(format!("{n}.m")), format!("{}\n", code.trim_end()));
        m.insert("file".into(), json!(format!("queries/{n}.m")));
        qmeta.push(Value::Object(m));
    }
    files.insert(sm.join("queries.json"), pretty(&Value::Array(qmeta)));
    files.insert(sm.join("parameters.json"), pretty(&take(&mut qdoc, "parameters")));

    // ---------------------------------------------------------------- report
    let rp = dir("Report");
    let mut report = match take(&mut root, "report") {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    let pages = arr(take(&mut report, "pages"));
    // Folders and files are named by id (like PBIR), so renaming or reordering a page or visual
    // changes one line of JSON instead of moving files.
    let pnames = unique(&pages.iter().map(|p| format!("page-{}", p.get("id").and_then(Value::as_u64).unwrap_or(0))).collect::<Vec<_>>());
    for (pg, pn) in pages.into_iter().zip(&pnames) {
        let Value::Object(mut pm) = pg else { continue };
        let elements = arr(take(&mut pm, "elements"));
        let enames = unique(&elements.iter().map(|e| format!("{}", e.get("id").and_then(Value::as_u64).unwrap_or(0))).collect::<Vec<_>>());
        for (e, en) in elements.iter().zip(&enames) {
            files.insert(rp.join("pages").join(pn).join("visuals").join(format!("{en}.json")), pretty(e));
        }
        pm.insert("visualOrder".into(), json!(enames));
        files.insert(rp.join("pages").join(pn).join("page.json"), pretty(&Value::Object(pm)));
    }
    report.insert("pageOrder".into(), json!(pnames));
    files.insert(rp.join("report.json"), pretty(&Value::Object(report)));

    // ---------------------------------------------------------------- data
    let dt = dir("Data");
    let sheets = arr(take(&mut root, "sheets"));
    let snames = unique(&sheets.iter().map(|s| s_of(s, "name")).collect::<Vec<_>>());
    for (sh, sn) in sheets.into_iter().zip(&snames) {
        let Value::Object(mut m) = sh else { continue };
        let cells = arr(take(&mut m, "cells"));
        files.insert(dt.join("sheets").join(format!("{sn}.csv")), to_csv(&cells));
        files.insert(dt.join("sheets").join(format!("{sn}.json")), pretty(&Value::Object(m)));
    }
    let mut data_index = Map::new();
    data_index.insert("sheetOrder".into(), json!(snames));
    for (key, sub) in [("flows", "flows"), ("notebooks", "notebooks")] {
        let items = arr(take(&mut root, key));
        let names = unique(&items.iter().map(|s| s_of(s, "name")).collect::<Vec<_>>());
        for (it, n) in items.iter().zip(&names) {
            files.insert(dt.join(sub).join(format!("{n}.json")), pretty(it));
        }
        data_index.insert(format!("{key}Order"), json!(names));
    }
    files.insert(dt.join("data.json"), pretty(&Value::Object(data_index)));

    // ---------------------------------------------------------------- science
    let sc = dir("Science");
    let mut sci_index = Map::new();
    let wfs = arr(take(&mut root, "studio"));
    let wnames = unique(&wfs.iter().map(|s| s_of(s, "name")).collect::<Vec<_>>());
    for (w, n) in wfs.iter().zip(&wnames) {
        files.insert(sc.join("workflows").join(format!("{n}.json")), pretty(w));
    }
    sci_index.insert("workflowOrder".into(), json!(wnames));
    let models = arr(take(&mut root, "models"));
    let mnames = unique(&models.iter().map(|m| format!("{}-v{}", s_of(m, "name"), m.get("version").and_then(Value::as_u64).unwrap_or(0))).collect::<Vec<_>>());
    for (m, n) in models.iter().zip(&mnames) {
        files.insert(sc.join("models").join(format!("{n}.json")), pretty(m));
    }
    sci_index.insert("modelOrder".into(), json!(mnames));
    files.insert(sc.join("science.json"), pretty(&Value::Object(sci_index)));

    // ---------------------------------------------------------------- settings + pointer
    files.insert(dir("Settings").join("publish.json"), pretty(&take(&mut root, "publish")));
    let mut pointer = Map::new();
    pointer.insert("$schema".into(), json!("https://vizcraft.dev/schemas/vizp-1.0.json"));
    pointer.insert("vizcraftProject".into(), json!(FOLDER_VERSION));
    pointer.insert("parts".into(), json!(PARTS.iter().map(|p| format!("{stem}.{p}")).collect::<Vec<_>>()));
    // Everything left (name, formatVersion, idSeed, …) stays in the pointer.
    for (k, v) in root {
        pointer.insert(k, v);
    }
    files.insert(PathBuf::from(format!("{stem}.{EXT}")), pretty(&Value::Object(pointer)));
    files
}

/// Rebuilds a project from files (relative paths, as [`to_files`] names them).
pub fn from_files(files: &Files, stem: &str) -> crate::Result<Project> {
    let get = |rel: PathBuf| files.get(&rel).cloned();
    let json_at = |rel: PathBuf| -> crate::Result<Value> {
        match files.get(&rel) {
            Some(s) => serde_json::from_str(s).map_err(|e| err(format!("{}: {e}", rel.display()))),
            None => Ok(Value::Null),
        }
    };
    let dir = |part: &str| PathBuf::from(format!("{stem}.{part}"));
    let Value::Object(mut root) = json_at(PathBuf::from(format!("{stem}.{EXT}")))? else { return Err(err(format!("{stem}.{EXT} is missing or not an object"))) };
    root.remove("$schema");
    root.remove("vizcraftProject");
    root.remove("parts");
    // Files under a folder, by the folder's relative path.
    let under = |folder: &Path, ext: &str| -> Vec<PathBuf> { files.keys().filter(|k| k.parent() == Some(folder) && k.extension().is_some_and(|e| e == ext)).cloned().collect() };
    // Orders an item list by an "order" array of stems, then any extras alphabetically.
    let ordered = |folder: &Path, order: &Value, ext: &str| -> Vec<PathBuf> {
        let all = under(folder, ext);
        let mut out: Vec<PathBuf> = arr(order.clone()).iter().filter_map(|n| n.as_str()).map(|n| folder.join(format!("{n}.{ext}"))).filter(|p| files.contains_key(p)).collect();
        for p in all {
            if !out.contains(&p) {
                out.push(p);
            }
        }
        out
    };

    // ---------------------------------------------------------------- semantic model
    let sm = dir("SemanticModel");
    let mut model = match json_at(sm.join("model.json"))? {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    let torder = model.remove("tableOrder").unwrap_or(Value::Null);
    let mut tables = vec![];
    for p in ordered(&sm.join("tables"), &torder, "json") {
        tables.push(json_at(p)?);
    }
    model.insert("tables".into(), Value::Array(tables));
    for (key, sub) in [("measures", "measures"), ("calculatedColumns", "columns")] {
        let mut items = vec![];
        for it in arr(json_at(sm.join(format!("{sub}.json")))?) {
            let Value::Object(mut m) = it else { continue };
            let f = m.remove("file").and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
            let expr = get(sm.join(&f)).unwrap_or_default();
            m.insert("expression".into(), json!(expr.trim_end()));
            items.push(Value::Object(m));
        }
        model.insert(key.into(), Value::Array(items));
    }
    root.insert("model".into(), Value::Object(model));
    let mut queries = vec![];
    for it in arr(json_at(sm.join("queries.json"))?) {
        let Value::Object(mut m) = it else { continue };
        let f = m.remove("file").and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
        let code = get(sm.join(&f)).unwrap_or_default();
        m.insert("m".into(), json!(code.trim_end()));
        queries.push(Value::Object(m));
    }
    let params = json_at(sm.join("parameters.json"))?;
    root.insert("queries".into(), json!({"queries": queries, "parameters": if params.is_null() { json!([]) } else { params }}));

    // ---------------------------------------------------------------- report
    let rp = dir("Report");
    let mut report = match json_at(rp.join("report.json"))? {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    let porder = report.remove("pageOrder").unwrap_or(Value::Null);
    let mut page_dirs: Vec<String> = arr(porder).iter().filter_map(|v| v.as_str().map(String::from)).collect();
    let pages_root = rp.join("pages");
    for k in files.keys() {
        if let Ok(rest) = k.strip_prefix(&pages_root)
            && rest.components().count() == 2
            && rest.file_name().is_some_and(|f| f == "page.json")
            && let Some(d) = rest.components().next()
        {
            let d = d.as_os_str().to_string_lossy().to_string();
            if !page_dirs.contains(&d) {
                page_dirs.push(d);
            }
        }
    }
    let mut pages = vec![];
    for d in page_dirs {
        let pdir = pages_root.join(&d);
        let Value::Object(mut pm) = json_at(pdir.join("page.json"))? else { continue };
        let vorder = pm.remove("visualOrder").unwrap_or(Value::Null);
        let mut els = vec![];
        for p in ordered(&pdir.join("visuals"), &vorder, "json") {
            els.push(json_at(p)?);
        }
        pm.insert("elements".into(), Value::Array(els));
        pages.push(Value::Object(pm));
    }
    report.insert("pages".into(), Value::Array(pages));
    root.insert("report".into(), Value::Object(report));

    // ---------------------------------------------------------------- data
    let dt = dir("Data");
    let index = json_at(dt.join("data.json"))?;
    let mut sheets = vec![];
    for p in ordered(&dt.join("sheets"), index.get("sheetOrder").unwrap_or(&Value::Null), "json") {
        let Value::Object(mut m) = json_at(p.clone())? else { continue };
        let csv = get(p.with_extension("csv")).unwrap_or_default();
        m.insert("cells".into(), json!(parse_csv(&csv)));
        sheets.push(Value::Object(m));
    }
    root.insert("sheets".into(), Value::Array(sheets));
    for (key, sub) in [("flows", "flows"), ("notebooks", "notebooks")] {
        let mut items = vec![];
        for p in ordered(&dt.join(sub), index.get(format!("{key}Order")).unwrap_or(&Value::Null), "json") {
            items.push(json_at(p)?);
        }
        root.insert(key.into(), Value::Array(items));
    }

    // ---------------------------------------------------------------- science + settings
    let sc = dir("Science");
    let sidx = json_at(sc.join("science.json"))?;
    let mut wfs = vec![];
    for p in ordered(&sc.join("workflows"), sidx.get("workflowOrder").unwrap_or(&Value::Null), "json") {
        wfs.push(json_at(p)?);
    }
    root.insert("studio".into(), Value::Array(wfs));
    let mut models = vec![];
    for p in ordered(&sc.join("models"), sidx.get("modelOrder").unwrap_or(&Value::Null), "json") {
        models.push(json_at(p)?);
    }
    root.insert("models".into(), Value::Array(models));
    let publish = json_at(dir("Settings").join("publish.json"))?;
    if !publish.is_null() {
        root.insert("publish".into(), publish);
    }
    serde_json::from_value(Value::Object(root)).map_err(|e| err(format!("Couldn't read the project folder: {e}")))
}

fn stem_of(pointer: &Path) -> String {
    pointer.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Project".into())
}

/// Saves `p` as `pointer` (`…/Sales.vizp`) plus its folders. Unchanged files are not rewritten;
/// stale files inside the managed folders are deleted. Also writes `.gitignore` and `AGENTS.md`
/// next to the pointer when they don't exist yet.
pub fn save(p: &Project, pointer: &Path) -> crate::Result<usize> {
    let base = pointer.parent().filter(|d| !d.as_os_str().is_empty()).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    let stem = stem_of(pointer);
    let files = to_files(p, &stem);
    let mut written = 0;
    for (rel, content) in &files {
        let path = base.join(rel);
        if std::fs::read_to_string(&path).is_ok_and(|old| old == *content) {
            continue;
        }
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| err(format!("{}: {e}", d.display())))?;
        }
        std::fs::write(&path, content).map_err(|e| err(format!("{}: {e}", path.display())))?;
        written += 1;
    }
    // Remove files the project no longer has (only our extensions, only inside our folders).
    for part in PARTS {
        let d = base.join(format!("{stem}.{part}"));
        for f in walk(&d) {
            let rel = f.strip_prefix(&base).map(Path::to_path_buf).unwrap_or_default();
            let ours = f.extension().is_some_and(|e| ["json", "dax", "m", "csv"].contains(&e.to_string_lossy().as_ref()));
            if ours && !files.contains_key(&rel) {
                let _ = std::fs::remove_file(&f);
            }
        }
        remove_empty_dirs(&d);
    }
    for (name, content) in [(".gitignore", GITIGNORE), ("AGENTS.md", AGENTS_MD), (".vscode/settings.json", VSCODE_SETTINGS), (".vscode/tasks.json", VSCODE_TASKS), (".mcp.json", MCP_JSON)] {
        let f = base.join(name);
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if !f.exists() {
            let _ = std::fs::write(f, content.replace("{stem}", &stem));
        }
    }
    Ok(written)
}

/// Loads a project from its pointer file (or from a folder holding exactly one pointer file).
pub fn load(path: &Path) -> crate::Result<Project> {
    let pointer = if path.is_dir() { find_pointer(path)? } else { path.to_path_buf() };
    let base = pointer.parent().filter(|d| !d.as_os_str().is_empty()).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    let stem = stem_of(&pointer);
    let mut files = Files::new();
    files.insert(PathBuf::from(format!("{stem}.{EXT}")), std::fs::read_to_string(&pointer).map_err(|e| err(format!("{}: {e}", pointer.display())))?);
    for part in PARTS {
        for f in walk(&base.join(format!("{stem}.{part}"))) {
            if let (Ok(rel), Ok(s)) = (f.strip_prefix(&base), std::fs::read_to_string(&f)) {
                files.insert(rel.to_path_buf(), s);
            }
        }
    }
    from_files(&files, &stem)
}

/// The single `.vizp` file in a folder.
pub fn find_pointer(dir: &Path) -> crate::Result<PathBuf> {
    let found: Vec<PathBuf> = std::fs::read_dir(dir).map_err(|e| err(format!("{}: {e}", dir.display())))?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|e| e == EXT)).collect();
    match found.len() {
        1 => Ok(found[0].clone()),
        0 => Err(err(format!("No .{EXT} project in {}", dir.display()))),
        _ => Err(err(format!("Several .{EXT} projects in {}: name one", dir.display()))),
    }
}

/// Newest modification time of the pointer and its folders (to notice outside edits).
pub fn last_modified(pointer: &Path) -> Option<std::time::SystemTime> {
    let base = pointer.parent()?.to_path_buf();
    let stem = stem_of(pointer);
    let mut newest = std::fs::metadata(pointer).and_then(|m| m.modified()).ok()?;
    for part in PARTS {
        for f in walk(&base.join(format!("{stem}.{part}"))) {
            if let Ok(t) = std::fs::metadata(&f).and_then(|m| m.modified())
                && t > newest
            {
                newest = t;
            }
        }
    }
    Some(newest)
}

/// Opens either format: `.vizp` (folder project) or `.vizc` / `.json` (single file).
pub fn open_any(path: &Path) -> crate::Result<Project> {
    if path.is_dir() || path.extension().is_some_and(|e| e == EXT) {
        load(path)
    } else {
        let s = std::fs::read_to_string(path).map_err(|e| err(format!("{}: {e}", path.display())))?;
        Project::from_json(&s)
    }
}

/// Saves in the format the extension names (`.vizp` folder project, else single `.vizc` file).
pub fn save_any(p: &Project, path: &Path) -> crate::Result<()> {
    if path.extension().is_some_and(|e| e == EXT) {
        save(p, path).map(|_| ())
    } else {
        std::fs::write(path, p.to_json()).map_err(|e| err(format!("{}: {e}", path.display())))
    }
}

fn walk(d: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    let Ok(rd) = std::fs::read_dir(d) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out.sort();
    out
}

fn remove_empty_dirs(d: &Path) {
    let Ok(rd) = std::fs::read_dir(d) else { return };
    for e in rd.flatten() {
        if e.path().is_dir() {
            remove_empty_dirs(&e.path());
            let _ = std::fs::remove_dir(e.path());
        }
    }
}

const GITIGNORE: &str = "# VizCraft: local caches never belong in Git
.vizcraft/
*.duckdb
*.duckdb.wal
";

const VSCODE_SETTINGS: &str = r#"{
  "files.associations": {
    "*.dax": "dax",
    "*.m": "powerquery",
    "*.vizp": "json"
  },
  "files.trimTrailingWhitespace": false
}
"#;

const VSCODE_TASKS: &str = r#"{
  "version": "2.0.0",
  "tasks": [
    {
      "label": "VizCraft: validate project",
      "type": "shell",
      "command": "vizcraft-server validate {stem}.vizp",
      "problemMatcher": [],
      "group": "test"
    }
  ]
}
"#;

/// MCP config picked up by Claude Code (and similar agents) opened in this folder.
const MCP_JSON: &str = r#"{
  "mcpServers": {
    "vizcraft": {
      "command": "vizcraft-server",
      "args": ["mcp", "{stem}.vizp"]
    }
  }
}
"#;

const AGENTS_MD: &str = r#"# {stem} — a VizCraft project

This folder is a VizCraft project in the **folder format** (`{stem}.vizp`), made for Git, code
review and AI agents. Open it in VizCraft (File → Open, pick `{stem}.vizp`), or edit the files.

| What | Where |
| --- | --- |
| Measures (DAX) | `{stem}.SemanticModel/measures/<Table>/<Name>.dax` (+ `measures.json` for format, folder…) |
| Calculated columns | `{stem}.SemanticModel/columns/<Table>/<Name>.dax` |
| Power Query (M) | `{stem}.SemanticModel/queries/<Query>.m` (+ `queries.json`) |
| Tables, relationships | `{stem}.SemanticModel/tables/*.json`, `model.json` |
| Report pages, visuals | `{stem}.Report/pages/page-<id>/page.json` (has the page `name`), `visuals/<id>.json` |
| Entered data | `{stem}.Data/sheets/<Sheet>.csv` |
| ML workflows, models | `{stem}.Science/` |

Rules for agents:
- Keep JSON valid; keep `id` values unique (new ids: larger than `idSeed` in `{stem}.vizp`, then raise `idSeed`).
- To add a measure: add an entry to `measures.json` (`table`, `name`, `format`, `file`) and its `.dax` file.
- Check your edits: `vizcraft-server validate {stem}.vizp` (loads, refreshes and evaluates every measure).
- Make changes with the same tools as the in-app Copilot: `vizcraft-server tools` lists them,
  `vizcraft-server tool {stem}.vizp add_measure '{"name":"Margin %","dax":"DIVIDE([Profit],[Sales])"}'` runs one and saves.
- Agents that speak MCP (Claude Code, VS Code, Cursor) get every Copilot tool through
  `vizcraft-server mcp {stem}.vizp` (already configured in `.mcp.json`).
- VizCraft reloads the project when these files change on disk (if it has no unsaved edits).
- `vizcraft-server` ships inside the app: on macOS `/Applications/VizCraft.app/Contents/MacOS/vizcraft-server`.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_the_sample_through_files_and_disk() {
        let mut p = crate::sample::sales();
        crate::build::sync_model_tables(&mut p);
        p.sheets.push(crate::doc::DataSheet { name: "Notes, \"quoted\"".into(), cells: vec![vec!["A".into(), "B".into()], vec!["x, y".into(), "line\nbreak".into()], vec!["\"q\"".into(), " pad ".into()]], ..Default::default() });
        let files = to_files(&p, "Sales");
        assert!(files.keys().any(|k| k.extension().is_some_and(|e| e == "dax")));
        assert!(files.keys().any(|k| k.extension().is_some_and(|e| e == "m")));
        let back = from_files(&files, "Sales").expect("load");
        assert_eq!(serde_json::to_value(&back).unwrap(), serde_json::to_value(&p).unwrap());

        let dir = std::env::temp_dir().join(format!("vizp-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ptr = dir.join("Sales.vizp");
        assert!(save(&p, &ptr).unwrap() > 5);
        assert_eq!(save(&p, &ptr).unwrap(), 0, "unchanged files are not rewritten");
        let loaded = load(&ptr).unwrap();
        assert_eq!(serde_json::to_value(&loaded).unwrap(), serde_json::to_value(&p).unwrap());
        // Edit a measure on disk like an agent would.
        let m = &p.model.measures[0];
        let f = dir.join("Sales.SemanticModel/measures").join(safe_name(&m.table)).join(format!("{}.dax", safe_name(&m.name)));
        std::fs::write(&f, "1 + 1\n").unwrap();
        assert_eq!(load(&dir).unwrap().model.measures[0].expression, "1 + 1");
        // Removing a page removes its folder on the next save.
        let mut p2 = loaded.clone();
        p2.report.pages.truncate(1);
        save(&p2, &ptr).unwrap();
        assert_eq!(load(&ptr).unwrap().report.pages.len(), 1);
        assert!(dir.join("AGENTS.md").exists() && dir.join(".gitignore").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
