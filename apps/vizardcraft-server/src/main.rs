//! `vizardcraft-server`: the VizardCraft Service portal, and command-line publishing.
//!
//! ```text
//! vizardcraft-server serve   [--port 8080] [--host 0.0.0.0] [--data ./vizardcraft-data] [--token SECRET] [--name "VizardCraft Service"]
//! vizardcraft-server publish <file.vizc> --to http://host:8080 [--workspace W] [--name N] [--token T] [--static] [--no-mobile]
//! vizardcraft-server export  <file.vizc> --out DIR [--single] [--static] [--no-mobile]
//! vizardcraft-server render  <file.vizc> [--page N] --svg out.svg [--mobile]
//! vizardcraft-server convert <in.vizc|in.vizp> <out.vizp|out.vizc>
//! vizardcraft-server validate <project>
//! vizardcraft-server describe <project>
//! vizardcraft-server dax     <project> "EVALUATE …"
//! vizardcraft-server tools
//! vizardcraft-server tool    <project> <tool> '<json args>' [--dry-run]
//! ```
//! Projects are `.vizc` (one file) or `.vizp` (Git-friendly folder format).
//! The token can also come from `VIZARDCRAFT_TOKEN`.

mod portal;
mod server;
mod store;

use std::collections::BTreeMap;
use std::path::PathBuf;

use vizardcraft_core::build::{self, Built};
use vizardcraft_core::doc::Project;
use vizardcraft_publish::BundleOptions;

const USAGE: &str = "VizardCraft Service

USAGE:
  vizardcraft-server serve   [--port 8080] [--host 0.0.0.0] [--data ./vizardcraft-data] [--token SECRET] [--name NAME]
  vizardcraft-server publish <file.vizc> --to URL [--workspace W] [--name N] [--token T] [--static] [--no-mobile]
  vizardcraft-server export  <file.vizc> --out DIR [--single] [--static] [--no-mobile]
  vizardcraft-server render  <file.vizc> [--page N] --svg OUT.svg [--mobile]

For agents, scripts and CI (projects can be .vizc files or .vizp folder projects):
  vizardcraft-server convert  IN OUT            .vizc ⇄ .vizp (Git-friendly folder, like Power BI PBIP)
  vizardcraft-server validate PROJECT           load, refresh, evaluate every measure; exit 1 on errors
  vizardcraft-server describe PROJECT           the model and report as JSON
  vizardcraft-server dax      PROJECT QUERY     run a DAX query (EVALUATE …), print JSON rows
  vizardcraft-server tools                      list the Copilot tools (JSON schemas)
  vizardcraft-server tool     PROJECT NAME ARGS run one Copilot tool and save (ARGS = JSON; --dry-run to not save)
  vizardcraft-server mcp      PROJECT           an MCP server on stdio (Claude Code, VS Code, Cursor…): the Copilot
                                             tools on this project; changes are saved as they happen

Use --sample instead of a file for the built-in retail sample.
The token can also be set with the VIZARDCRAFT_TOKEN environment variable.
Pages are numbered from 1 in `render`.";

/// Parsed `--flag value` options and positional arguments.
struct Args {
    pos: Vec<String>,
    opts: BTreeMap<String, String>,
    flags: Vec<String>,
}

impl Args {
    fn parse(raw: &[String]) -> Args {
        const SWITCHES: &[&str] = &["single", "static", "no-mobile", "mobile", "help", "sample", "dry-run"];
        let mut a = Args { pos: vec![], opts: BTreeMap::new(), flags: vec![] };
        let mut i = 0;
        while let Some(s) = raw.get(i) {
            if let Some(k) = s.strip_prefix("--") {
                if let Some((k, v)) = k.split_once('=') {
                    a.opts.insert(k.to_string(), v.to_string());
                } else if SWITCHES.contains(&k) {
                    a.flags.push(k.to_string());
                } else if let Some(v) = raw.get(i + 1) {
                    a.opts.insert(k.to_string(), v.clone());
                    i += 1;
                } else {
                    a.flags.push(k.to_string());
                }
            } else if s == "-h" {
                a.flags.push("help".into());
            } else {
                a.pos.push(s.clone());
            }
            i += 1;
        }
        a
    }
    fn get(&self, k: &str) -> Option<&str> {
        self.opts.get(k).map(String::as_str)
    }
    fn flag(&self, k: &str) -> bool {
        self.flags.iter().any(|f| f == k)
    }
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let cmd = raw.first().cloned().unwrap_or_default();
    let args = Args::parse(raw.get(1..).unwrap_or(&[]));
    let result = match cmd.as_str() {
        "serve" => serve(&args),
        "publish" => publish(&args),
        "export" => export(&args),
        "render" => render(&args),
        "convert" => convert(&args),
        "validate" => validate(&args),
        "describe" => describe(&args),
        "dax" => dax(&args),
        "tools" => {
            let specs: Vec<serde_json::Value> = vizardcraft_ai::tools::specs().into_iter().map(|t| serde_json::json!({"name": t.name, "description": t.description, "parameters": t.parameters, "mutating": vizardcraft_ai::tools::is_mutating(&t.name)})).collect();
            println!("{}", serde_json::to_string_pretty(&specs).unwrap_or_default());
            Ok(())
        }
        "tool" => tool(&args),
        "mcp" => mcp(&args),
        "" | "help" | "--help" | "-h" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("Unknown command `{other}`.\n\n{USAGE}")),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn token(args: &Args) -> String {
    args.get("token").map(String::from).or_else(|| std::env::var("VIZARDCRAFT_TOKEN").ok()).unwrap_or_default()
}

fn serve(args: &Args) -> Result<(), String> {
    let port: u16 = args.get("port").map(|p| p.parse().map_err(|_| format!("`{p}` isn't a port number."))).transpose()?.unwrap_or(8080);
    let host = args.get("host").unwrap_or("0.0.0.0");
    let data = PathBuf::from(args.get("data").unwrap_or("./vizardcraft-data"));
    let cfg = server::Config { data, token: token(args), name: args.get("name").unwrap_or("VizardCraft Service").to_string() };
    server::run(&format!("{host}:{port}"), cfg)
}

/// `--sample` stands for the built-in retail sample instead of a file.
const SAMPLE: &str = "\u{0}sample";

fn project_arg(args: &Args, usage: &str) -> Result<String, String> {
    if args.flag("sample") {
        return Ok(SAMPLE.to_string());
    }
    args.pos.first().cloned().ok_or_else(|| format!("Which project? {usage}"))
}

/// Opens a saved project and refreshes its data (queries, files, databases).
fn load(path: &str) -> Result<(Project, Built), String> {
    let mut p = if path == SAMPLE {
        vizardcraft_core::sample::sales()
    } else {
        vizardcraft_core::project_folder::open_any(std::path::Path::new(path)).map_err(|e| format!("Couldn't read {path}: {e}"))?
    };
    // Relative file paths in queries resolve next to the project.
    if let Some(dir) = std::path::Path::new(path).parent().filter(|d| !d.as_os_str().is_empty()) {
        let _ = std::env::set_current_dir(dir);
    }
    build::sync_model_tables(&mut p);
    let params = p.model.parameters.iter().map(|x| (x.name.clone(), x.current.clone())).collect();
    let host: vizardcraft_connectors::DesktopHost = Default::default();
    let built = build::build(&p, &host, &params);
    for (obj, e) in &built.errors {
        eprintln!("warning: {obj}: {e}");
    }
    Ok((p, built))
}

fn abs(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_absolute() { p } else { std::env::current_dir().map(|d| d.join(&p)).unwrap_or(p) }
}

fn convert(args: &Args) -> Result<(), String> {
    let (p, dst) = if args.flag("sample") {
        (vizardcraft_core::sample::sales(), args.pos.first())
    } else {
        let Some(src) = args.pos.first() else { return Err("Usage: vizardcraft-server convert IN OUT".into()) };
        (vizardcraft_core::project_folder::open_any(&abs(src)).map_err(|e| e.to_string())?, args.pos.get(1))
    };
    let Some(dst) = dst else { return Err("Usage: vizardcraft-server convert IN OUT".into()) };
    vizardcraft_core::project_folder::save_any(&p, &abs(dst)).map_err(|e| e.to_string())?;
    println!("Wrote {dst}");
    Ok(())
}

fn validate(args: &Args) -> Result<(), String> {
    let path = project_arg(args, "vizardcraft-server validate PROJECT")?;
    let (p, built) = load(&path)?;
    let mut problems: Vec<String> = built.errors.iter().map(|(o, e)| format!("{o}: {e}")).collect();
    for m in &p.model.measures {
        if let Err(e) = built.model.evaluate(&format!("[{}]", m.name)) {
            problems.push(format!("measure [{}]: {e}", m.name));
        }
    }
    let mut ids = std::collections::BTreeSet::new();
    for pg in &p.report.pages {
        for e in &pg.elements {
            if !ids.insert(e.id) {
                problems.push(format!("duplicate element id {} on page {}", e.id, pg.name));
            }
            if e.id > p.id_seed {
                problems.push(format!("element id {} is above idSeed {} (raise idSeed)", e.id, p.id_seed));
            }
        }
    }
    let summary = serde_json::json!({"ok": problems.is_empty(), "tables": built.tables.len(), "measures": p.model.measures.len(), "pages": p.report.pages.len(), "problems": problems});
    println!("{}", serde_json::to_string_pretty(&summary).unwrap_or_default());
    if problems.is_empty() { Ok(()) } else { Err(format!("{} problem(s)", problems.len())) }
}

fn describe(args: &Args) -> Result<(), String> {
    let path = project_arg(args, "vizardcraft-server describe PROJECT")?;
    let (p, built) = load(&path)?;
    println!("{}", serde_json::to_string_pretty(&vizardcraft_ai::tools::describe_model_json(&p, &built)).unwrap_or_default());
    Ok(())
}

fn dax(args: &Args) -> Result<(), String> {
    let path = project_arg(args, "vizardcraft-server dax PROJECT QUERY")?;
    let q = args.pos.get(if path == SAMPLE { 0 } else { 1 }).cloned().ok_or("Which DAX query?")?;
    let (p, built) = load(&path)?;
    let (cols, rows) = vizardcraft_ai::tools::run_dax_query(&built, &p, &q)?;
    let out: Vec<serde_json::Value> = rows.iter().map(|r| serde_json::Value::Object(cols.iter().zip(r).map(|(c, v)| (c.clone(), serde_json::to_value(v).unwrap_or_default())).collect())).collect();
    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    Ok(())
}

fn tool(args: &Args) -> Result<(), String> {
    let usage = "vizardcraft-server tool PROJECT NAME '{json args}' [--dry-run]";
    let path = project_arg(args, usage)?;
    let off = if path == SAMPLE { 0 } else { 1 };
    let name = args.pos.get(off).cloned().ok_or(format!("Which tool? {usage}"))?;
    let raw = args.pos.get(off + 1).cloned().unwrap_or_else(|| "{}".into());
    let arguments: serde_json::Value = serde_json::from_str(&raw).map_err(|e| format!("Arguments must be JSON: {e}"))?;
    let save_path = (path != SAMPLE).then(|| abs(&path));
    let (mut p, mut built) = load(&path)?;
    let mut page = p.report.pages.first().map(|x| x.id).unwrap_or(0);
    let out = {
        let mut ws = vizardcraft_ai::tools::Workspace { project: &mut p, built: &mut built, page: &mut page, selected: None, max_rows: 200, send_rows: true };
        vizardcraft_ai::tools::execute(&mut ws, &vizardcraft_ai::provider::ToolCall { id: "cli".into(), name: name.clone(), arguments })
    };
    let result: serde_json::Value = serde_json::from_str(&out.text).unwrap_or(serde_json::Value::String(out.text.clone()));
    let mut saved = false;
    if out.ok && out.mutated && !args.flag("dry-run") {
        if let Some(sp) = &save_path {
            vizardcraft_core::project_folder::save_any(&p, sp).map_err(|e| e.to_string())?;
            saved = true;
        }
    }
    println!("{}", serde_json::to_string_pretty(&serde_json::json!({"ok": out.ok, "summary": out.summary, "saved": saved, "result": result})).unwrap_or_default());
    if out.ok { Ok(()) } else { Err(out.summary) }
}

/// A Model Context Protocol server over stdio (JSON-RPC 2.0, one message per line): exposes the
/// Copilot tools on one project. Mutating calls save the project immediately, so a VizardCraft window
/// with the project open reloads it.
fn mcp(args: &Args) -> Result<(), String> {
    use std::io::{BufRead, Write};
    use serde_json::{Value, json};
    let path = project_arg(args, "vizardcraft-server mcp PROJECT")?;
    let save_path = (path != SAMPLE).then(|| abs(&path));
    let (mut p, mut built) = load(&path)?;
    let mut page = p.report.pages.first().map(|x| x.id).unwrap_or(0);
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let result: Result<Value, (i64, String)> = match method {
            "initialize" => Ok(json!({
                "protocolVersion": msg.pointer("/params/protocolVersion").cloned().unwrap_or(json!("2025-06-18")),
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "vizardcraft", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Tools to read and edit a VizardCraft analytics project (semantic model, DAX, Power Query M, report pages and visuals, data sheets, ML workflows). Call describe_model or get_project first. Every change is validated and saved."
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": vizardcraft_ai::tools::specs().into_iter().filter(|t| !matches!(t.name.as_str(), "navigate" | "app_command")).map(|t| json!({"name": t.name, "description": t.description, "inputSchema": t.parameters})).collect::<Vec<_>>()})),
            "tools/call" => {
                let name = msg.pointer("/params/name").and_then(Value::as_str).unwrap_or("").to_string();
                let arguments = msg.pointer("/params/arguments").cloned().unwrap_or(json!({}));
                let o = {
                    let mut ws = vizardcraft_ai::tools::Workspace { project: &mut p, built: &mut built, page: &mut page, selected: None, max_rows: 200, send_rows: true };
                    vizardcraft_ai::tools::execute(&mut ws, &vizardcraft_ai::provider::ToolCall { id: "mcp".into(), name, arguments })
                };
                let mut text = o.text.clone();
                if o.ok && o.mutated && let Some(sp) = &save_path {
                    if let Err(e) = vizardcraft_core::project_folder::save_any(&p, sp) {
                        text = format!("{text}\n(warning: not saved: {e})");
                    }
                }
                Ok(json!({"content": [{"type": "text", "text": text}], "isError": !o.ok}))
            }
            m if m.starts_with("notifications/") => continue,
            other => Err((-32601, format!("Unknown method {other}"))),
        };
        let Some(id) = id else { continue };
        let reply = match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err((code, m)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": m}}),
        };
        writeln!(out, "{reply}").map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn bundle_opts(p: &Project, args: &Args) -> BundleOptions {
    let mut o = BundleOptions::from_project(p);
    o.interactive = !args.flag("static");
    o.include_mobile = !args.flag("no-mobile");
    if o.interactive && !vizardcraft_publish::has_wasm() {
        eprintln!("note: this build has no WASM engine (run packaging/build-wasm.sh, then rebuild); publishing a static report.");
    }
    o
}

fn absolute(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    if p.is_absolute() { Ok(p) } else { std::env::current_dir().map(|d| d.join(p)).map_err(|e| e.to_string()) }
}

fn publish(args: &Args) -> Result<(), String> {
    let file = &project_arg(args, "vizardcraft-server publish <file.vizc> --to URL")?;
    let url = args.get("to").ok_or("Where to? Add --to http://host:8080")?.to_string();
    let (p, built) = load(file)?;
    let opts = bundle_opts(&p, args);
    let files = vizardcraft_publish::bundle(&p, &built, &opts)?;
    let workspace = args.get("workspace").unwrap_or("My workspace");
    let name = args.get("name").map(String::from).unwrap_or_else(|| p.name.clone());
    let link = vizardcraft_publish::net::publish_to_service(&url, workspace, &name, &token(args), &files)?;
    println!("Published {name} to {workspace}: {link}");
    Ok(())
}

fn export(args: &Args) -> Result<(), String> {
    let file = &project_arg(args, "vizardcraft-server export <file.vizc> --out DIR")?;
    let out = absolute(args.get("out").ok_or("Add --out DIR (or --out report.html with --single)")?)?;
    let (p, built) = load(file)?;
    let opts = bundle_opts(&p, args);
    if args.flag("single") {
        let html = vizardcraft_publish::single_file(&p, &built, &opts)?;
        let path = if out.extension().is_some_and(|e| e == "html" || e == "htm") { out } else { out.join("report.html") };
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, html).map_err(|e| e.to_string())?;
        println!("Wrote {}", path.display());
    } else {
        let files = vizardcraft_publish::bundle(&p, &built, &opts)?;
        vizardcraft_publish::net::write_folder(&files, &out)?;
        println!("Wrote {} files to {}", files.len(), out.display());
    }
    Ok(())
}

fn render(args: &Args) -> Result<(), String> {
    let file = &project_arg(args, "vizardcraft-server render <file.vizc> --page 1 --svg out.svg")?;
    let out = absolute(args.get("svg").ok_or("Add --svg out.svg")?)?;
    let (p, built) = load(file)?;
    let n: usize = args.get("page").map(|s| s.parse().map_err(|_| format!("`{s}` isn't a page number."))).transpose()?.unwrap_or(1);
    let page = p.report.pages.get(n.saturating_sub(1)).ok_or_else(|| format!("The project has {} pages.", p.report.pages.len()))?;
    let rt = vizardcraft_core::state::Runtime { page: page.id, ..Default::default() };
    let svg = vizardcraft_publish::page_svg(&p, &built, &rt, page, args.flag("mobile"));
    std::fs::write(&out, svg).map_err(|e| e.to_string())?;
    println!("Wrote {} ({})", out.display(), page.name);
    Ok(())
}
