//! The HTTP side of the service (tiny_http, a few worker threads, no async runtime).

use std::io::Read as _;
use std::path::PathBuf;
use std::sync::Arc;

use tiny_http::{Header, Method, Request, Response, Server};

use crate::portal;
use crate::store::{Meta, Store};

pub struct Config {
    pub data: PathBuf,
    /// Bearer token needed to publish and delete ("" = anyone may publish: local use only).
    pub token: String,
    pub name: String,
}

pub struct Ctx {
    pub cfg: Config,
    pub store: Store,
}

const MAX_UPLOAD: usize = 512 * 1024 * 1024;

pub fn run(addr: &str, cfg: Config) -> Result<(), String> {
    let store = Store::new(cfg.data.clone())?;
    let server = Server::http(addr).map_err(|e| format!("Couldn't listen on {addr}: {e}"))?;
    println!("{} is running on http://{addr}  (data: {})", cfg.name, cfg.data.display());
    if cfg.token.is_empty() {
        println!("warning: no --token set; anyone who can reach this port can publish and delete reports.");
    }
    let ctx = Arc::new(Ctx { cfg, store });
    let server = Arc::new(server);
    let mut workers = Vec::new();
    for _ in 0..6 {
        let s = server.clone();
        let c = ctx.clone();
        workers.push(std::thread::spawn(move || {
            for req in s.incoming_requests() {
                handle(&c, req);
            }
        }));
    }
    for w in workers {
        let _ = w.join();
    }
    Ok(())
}

fn header(k: &str, v: &str) -> Option<Header> {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).ok()
}

fn respond(req: Request, status: u16, ctype: &str, body: Vec<u8>) {
    let mut r = Response::from_data(body).with_status_code(status);
    for h in [header("Content-Type", ctype), header("X-Content-Type-Options", "nosniff"), header("Cache-Control", "no-cache")].into_iter().flatten() {
        r.add_header(h);
    }
    let _ = req.respond(r);
}

fn json(req: Request, status: u16, v: &serde_json::Value) {
    respond(req, status, "application/json; charset=utf-8", v.to_string().into_bytes());
}

fn html(req: Request, status: u16, body: String) {
    respond(req, status, "text/html; charset=utf-8", body.into_bytes());
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn query(url: &str) -> Vec<(String, String)> {
    let q = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    q.split('&').filter(|s| !s.is_empty()).map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        (decode(k), decode(v))
    }).collect()
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        match c {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            _ => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn authorized(ctx: &Ctx, req: &Request) -> bool {
    if ctx.cfg.token.is_empty() {
        return true;
    }
    let given = req.headers().iter().find(|h| h.field.equiv("Authorization")).map(|h| h.value.as_str().trim().to_string()).unwrap_or_default();
    let given = given.strip_prefix("Bearer ").unwrap_or("").trim();
    // Constant-time comparison.
    let (a, b) = (given.as_bytes(), ctx.cfg.token.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn handle(ctx: &Ctx, mut req: Request) {
    let url = req.url().to_string();
    let path = decode(url.split('?').next().unwrap_or("/"));
    let parts: Vec<&str> = path.trim_matches('/').split('/').filter(|s| !s.is_empty()).collect();
    let method = req.method().clone();
    match (&method, parts.as_slice()) {
        (Method::Get, []) => html(req, 200, portal::home(ctx, None, &query(&url))),
        (Method::Get, ["w", ws]) => html(req, 200, portal::home(ctx, Some(ws), &query(&url))),
        (Method::Get, ["r", ws, slug]) => match ctx.store.get(ws, slug) {
            Some(m) => {
                let q = query(&url);
                let phone = q.iter().any(|(k, v)| k == "view" && v == "phone");
                html(req, 200, portal::report(ctx, &m, phone))
            }
            None => html(req, 404, portal::not_found(ctx)),
        },
        (Method::Get, ["r", ws, slug, "files", rest @ ..]) => {
            // The viewer loads relative files: make sure the folder URL ends with a slash.
            if rest.is_empty() && !path.ends_with('/') {
                let mut r = Response::from_data(Vec::new()).with_status_code(301);
                if let Some(h) = header("Location", &format!("/r/{ws}/{slug}/files/")) {
                    r.add_header(h);
                }
                let _ = req.respond(r);
                return;
            }
            let rel = rest.join("/");
            match ctx.store.file(ws, slug, &rel) {
                Some(f) => match std::fs::read(&f) {
                    Ok(b) => respond(req, 200, content_type(&f.to_string_lossy()), b),
                    Err(_) => html(req, 404, portal::not_found(ctx)),
                },
                None => html(req, 404, portal::not_found(ctx)),
            }
        }
        (Method::Get, ["thumb", ws, slug]) => {
            let slug = slug.trim_end_matches(".svg");
            match ctx.store.file(ws, slug, "thumbnail.svg").and_then(|f| std::fs::read(f).ok()) {
                Some(b) => respond(req, 200, "image/svg+xml", b),
                None => respond(req, 404, "text/plain", b"no thumbnail".to_vec()),
            }
        }
        (Method::Get, ["api", "ping"]) => {
            let can = authorized(ctx, &req);
            json(req, 200, &serde_json::json!({"name": ctx.cfg.name, "service": "vizcraft", "version": env!("CARGO_PKG_VERSION"), "reports": ctx.store.list().len(), "canPublish": can}));
        }
        (Method::Get, ["api", "reports"]) => {
            let list: Vec<Meta> = ctx.store.list();
            json(req, 200, &serde_json::json!({"reports": list}));
        }
        (Method::Post, ["api", "publish"]) => {
            if !authorized(ctx, &req) {
                json(req, 401, &serde_json::json!({"error": "A valid token is needed to publish (Authorization: Bearer …)."}));
                return;
            }
            let q = query(&url);
            let get = |k: &str| q.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();
            let workspace = if get("workspace").trim().is_empty() { "My workspace".to_string() } else { get("workspace") };
            let name = if get("name").trim().is_empty() { "Report".to_string() } else { get("name") };
            let mut body = Vec::new();
            if req.as_reader().take(MAX_UPLOAD as u64 + 1).read_to_end(&mut body).is_err() || body.len() > MAX_UPLOAD {
                json(req, 413, &serde_json::json!({"error": "The upload is too large or was cut off."}));
                return;
            }
            let files = if body.starts_with(b"PK") {
                vizcraft_publish::net::unzip_files(&body)
            } else {
                files_from_json(&body)
            };
            match files.and_then(|f| ctx.store.put(&workspace, &name, &f)) {
                Ok(m) => {
                    println!("published {} / {}", m.workspace, m.name);
                    json(req, 201, &serde_json::json!({"ok": true, "url": m.url, "report": m}));
                }
                Err(e) => json(req, 400, &serde_json::json!({"error": e})),
            }
        }
        (Method::Delete, ["api", "reports", ws, slug]) => {
            if !authorized(ctx, &req) {
                json(req, 401, &serde_json::json!({"error": "A valid token is needed to delete reports."}));
                return;
            }
            match ctx.store.delete(ws, slug) {
                Ok(()) => json(req, 200, &serde_json::json!({"ok": true})),
                Err(e) => json(req, 404, &serde_json::json!({"error": e})),
            }
        }
        _ => html(req, 404, portal::not_found(ctx)),
    }
}

/// `{"files": {"index.html": "<base64>", …}}`.
fn files_from_json(body: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let j: serde_json::Value = serde_json::from_slice(body).map_err(|e| format!("Send a zip or JSON: {e}"))?;
    let obj = j.get("files").and_then(|f| f.as_object()).ok_or("The JSON needs a `files` object (path → base64).")?;
    obj.iter().map(|(k, v)| v.as_str().and_then(vizcraft_publish::b64_decode).map(|b| (k.clone(), b)).ok_or_else(|| format!("{k} isn't base64."))).collect()
}
