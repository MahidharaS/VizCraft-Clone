//! `vizcraft-mlserve`: serves the models registered in VizCraft (Data Science Studio → Deploy)
//! over HTTP. `POST /models/<name>/predict` scores rows; `GET /models` lists what is served.
//!
//! Usage: `vizcraft-mlserve [--dir <registry>] [--host 127.0.0.1] [--port 8765] [--list]`

use vizcraft_mlops::{Registry, server};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("vizcraft-mlserve — serve VizCraft models over HTTP\n\n  --dir <path>   model registry (default ~/.vizcraft/models)\n  --host <host>  bind address (default 127.0.0.1)\n  --port <port>  port (default 8765)\n  --list         list registered models and exit\n\nEndpoints: GET /models · GET /models/<name> · POST /models/<name>/predict");
        return;
    }
    let registry = match value("--dir") {
        Some(d) => Registry::new(d),
        None => Registry::open_default(),
    };
    if args.iter().any(|a| a == "--list") {
        for m in registry.list() {
            println!("{}\tv{}\t{:?}", m.name, m.version, m.stage);
        }
        return;
    }
    let host = value("--host").unwrap_or_else(|| "127.0.0.1".into());
    let port = value("--port").and_then(|p| p.parse().ok()).unwrap_or(8765);
    let base = format!("http://{host}:{port}");
    let models = registry.list();
    eprintln!("vizcraft-mlserve: {} model version(s) in the registry, listening on {base}", models.len());
    if let Some(m) = models.first() {
        eprintln!("try:\n{}", server::curl_example(&base, m));
    }
    if let Err(e) = server::run_forever(registry, &host, port) {
        eprintln!("vizcraft-mlserve: {e}");
        std::process::exit(1);
    }
}
