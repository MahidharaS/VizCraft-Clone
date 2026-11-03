//! `vizcraft-server`: the VizCraft Service portal, and command-line publishing.
//!
//! ```text
//! vizcraft-server serve   [--port 8080] [--host 0.0.0.0] [--data ./vizcraft-data] [--token SECRET] [--name "VizCraft Service"]
//! vizcraft-server publish <file.vizc> --to http://host:8080 [--workspace W] [--name N] [--token T] [--static] [--no-mobile]
//! vizcraft-server export  <file.vizc> --out DIR [--single] [--static] [--no-mobile]
//! vizcraft-server render  <file.vizc> [--page N] --svg out.svg [--mobile]
//! ```
//! The token can also come from `VIZCRAFT_TOKEN`.

mod portal;
mod server;
mod store;

use std::collections::BTreeMap;
use std::path::PathBuf;

use vizcraft_core::build::{self, Built};
use vizcraft_core::doc::Project;
use vizcraft_publish::BundleOptions;

const USAGE: &str = "VizCraft Service

USAGE:
  vizcraft-server serve   [--port 8080] [--host 0.0.0.0] [--data ./vizcraft-data] [--token SECRET] [--name NAME]
  vizcraft-server publish <file.vizc> --to URL [--workspace W] [--name N] [--token T] [--static] [--no-mobile]
  vizcraft-server export  <file.vizc> --out DIR [--single] [--static] [--no-mobile]
  vizcraft-server render  <file.vizc> [--page N] --svg OUT.svg [--mobile]

Use --sample instead of a file for the built-in retail sample.
The token can also be set with the VIZCRAFT_TOKEN environment variable.
Pages are numbered from 1 in `render`.";

/// Parsed `--flag value` options and positional arguments.
struct Args {
    pos: Vec<String>,
    opts: BTreeMap<String, String>,
    flags: Vec<String>,
}

impl Args {
    fn parse(raw: &[String]) -> Args {
        const SWITCHES: &[&str] = &["single", "static", "no-mobile", "mobile", "help", "sample"];
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
    args.get("token").map(String::from).or_else(|| std::env::var("VIZCRAFT_TOKEN").ok()).unwrap_or_default()
}

fn serve(args: &Args) -> Result<(), String> {
    let port: u16 = args.get("port").map(|p| p.parse().map_err(|_| format!("`{p}` isn't a port number."))).transpose()?.unwrap_or(8080);
    let host = args.get("host").unwrap_or("0.0.0.0");
    let data = PathBuf::from(args.get("data").unwrap_or("./vizcraft-data"));
    let cfg = server::Config { data, token: token(args), name: args.get("name").unwrap_or("VizCraft Service").to_string() };
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
        vizcraft_core::sample::sales()
    } else {
        let text = std::fs::read_to_string(path).map_err(|e| format!("Couldn't read {path}: {e}"))?;
        Project::from_json(&text).map_err(|e| e.to_string())?
    };
    // Relative file paths in queries resolve next to the project.
    if let Some(dir) = std::path::Path::new(path).parent().filter(|d| !d.as_os_str().is_empty()) {
        let _ = std::env::set_current_dir(dir);
    }
    build::sync_model_tables(&mut p);
    let params = p.model.parameters.iter().map(|x| (x.name.clone(), x.current.clone())).collect();
    let host: vizcraft_connectors::DesktopHost = Default::default();
    let built = build::build(&p, &host, &params);
    for (obj, e) in &built.errors {
        eprintln!("warning: {obj}: {e}");
    }
    Ok((p, built))
}

fn bundle_opts(p: &Project, args: &Args) -> BundleOptions {
    let mut o = BundleOptions::from_project(p);
    o.interactive = !args.flag("static");
    o.include_mobile = !args.flag("no-mobile");
    if o.interactive && !vizcraft_publish::has_wasm() {
        eprintln!("note: this build has no WASM engine (run packaging/build-wasm.sh, then rebuild); publishing a static report.");
    }
    o
}

fn absolute(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    if p.is_absolute() { Ok(p) } else { std::env::current_dir().map(|d| d.join(p)).map_err(|e| e.to_string()) }
}

fn publish(args: &Args) -> Result<(), String> {
    let file = &project_arg(args, "vizcraft-server publish <file.vizc> --to URL")?;
    let url = args.get("to").ok_or("Where to? Add --to http://host:8080")?.to_string();
    let (p, built) = load(file)?;
    let opts = bundle_opts(&p, args);
    let files = vizcraft_publish::bundle(&p, &built, &opts)?;
    let workspace = args.get("workspace").unwrap_or("My workspace");
    let name = args.get("name").map(String::from).unwrap_or_else(|| p.name.clone());
    let link = vizcraft_publish::net::publish_to_service(&url, workspace, &name, &token(args), &files)?;
    println!("Published {name} to {workspace}: {link}");
    Ok(())
}

fn export(args: &Args) -> Result<(), String> {
    let file = &project_arg(args, "vizcraft-server export <file.vizc> --out DIR")?;
    let out = absolute(args.get("out").ok_or("Add --out DIR (or --out report.html with --single)")?)?;
    let (p, built) = load(file)?;
    let opts = bundle_opts(&p, args);
    if args.flag("single") {
        let html = vizcraft_publish::single_file(&p, &built, &opts)?;
        let path = if out.extension().is_some_and(|e| e == "html" || e == "htm") { out } else { out.join("report.html") };
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, html).map_err(|e| e.to_string())?;
        println!("Wrote {}", path.display());
    } else {
        let files = vizcraft_publish::bundle(&p, &built, &opts)?;
        vizcraft_publish::net::write_folder(&files, &out)?;
        println!("Wrote {} files to {}", files.len(), out.display());
    }
    Ok(())
}

fn render(args: &Args) -> Result<(), String> {
    let file = &project_arg(args, "vizcraft-server render <file.vizc> --page 1 --svg out.svg")?;
    let out = absolute(args.get("svg").ok_or("Add --svg out.svg")?)?;
    let (p, built) = load(file)?;
    let n: usize = args.get("page").map(|s| s.parse().map_err(|_| format!("`{s}` isn't a page number."))).transpose()?.unwrap_or(1);
    let page = p.report.pages.get(n.saturating_sub(1)).ok_or_else(|| format!("The project has {} pages.", p.report.pages.len()))?;
    let rt = vizcraft_core::state::Runtime { page: page.id, ..Default::default() };
    let svg = vizcraft_publish::page_svg(&p, &built, &rt, page, args.flag("mobile"));
    std::fs::write(&out, svg).map_err(|e| e.to_string())?;
    println!("Wrote {} ({})", out.display(), page.name);
    Ok(())
}
