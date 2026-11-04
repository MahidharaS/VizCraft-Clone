//! A small HTTP/1.1 JSON scoring server (std only, one thread per connection).
//!
//! - `GET  /health` → `{"status":"ok"}`
//! - `GET  /models` → registered models with their versions, stages and serving version
//! - `GET  /models/<name>` → versions of one model
//! - `GET  /models/<name>/schema` → features (name, type, categories), target, classes
//! - `POST /models/<name>/predict` → body `{"rows":[{…}]}`, `[{…}]` or `{…}`; answers
//!   `{"model","version","predictions":[{"prediction":…,"probabilities":{…}}]}`
//! - `POST /models/<name>/versions/<v>/predict` → a specific version
//!
//! The registry directory is re-read when it changes, so newly registered or promoted versions
//! are served without a restart.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{Value as J, json};

use crate::{Registry, RegisteredModel};

/// Caches the registry listing, refreshed when files change (checked at most every second).
struct Cache {
    reg: Registry,
    models: Vec<RegisteredModel>,
    stamp: Option<(usize, SystemTime)>,
    checked: Option<Instant>,
}

impl Cache {
    fn stamp(&self) -> Option<(usize, SystemTime)> {
        let mut n = 0;
        let mut latest = SystemTime::UNIX_EPOCH;
        for d in std::fs::read_dir(&self.reg.dir).ok()?.flatten() {
            if let Ok(rd) = std::fs::read_dir(d.path()) {
                for f in rd.flatten() {
                    n += 1;
                    if let Ok(m) = f.metadata().and_then(|m| m.modified()) {
                        latest = latest.max(m);
                    }
                }
            }
        }
        Some((n, latest))
    }
    fn models(&mut self) -> &[RegisteredModel] {
        if self.checked.is_none_or(|c| c.elapsed() > Duration::from_millis(1000)) {
            self.checked = Some(Instant::now());
            let s = self.stamp();
            if s != self.stamp || self.models.is_empty() {
                self.models = self.reg.list();
                self.stamp = s;
            }
        }
        &self.models
    }
}

pub struct Response {
    pub status: u16,
    pub body: String,
    pub content_type: &'static str,
}

fn json_resp(status: u16, v: J) -> Response {
    Response { status, body: serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".into()), content_type: "application/json" }
}

fn err(status: u16, msg: &str) -> Response {
    json_resp(status, json!({ "error": msg }))
}

fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn summary(models: &[RegisteredModel]) -> J {
    let mut names: Vec<&str> = models.iter().map(|m| m.name.as_str()).collect();
    names.dedup();
    J::Array(
        names
            .iter()
            .map(|n| {
                let serving = crate::records::serving_version(models, n);
                json!({
                    "name": n,
                    "servingVersion": serving.map(|m| m.version),
                    "algorithm": serving.map(|m| m.algorithm.clone()),
                    "task": serving.map(|m| m.task.clone()),
                    "target": serving.map(|m| m.target.clone()),
                    "versions": models.iter().filter(|m| m.name == *n).map(|m| json!({"version": m.version, "stage": m.stage.label(), "created": m.created, "algorithm": m.algorithm, "metrics": m.metrics})).collect::<Vec<_>>(),
                    "predictUrl": format!("/models/{}/predict", n.replace(' ', "%20")),
                })
            })
            .collect(),
    )
}

fn schema(m: &RegisteredModel) -> J {
    json!({
        "name": m.name, "version": m.version, "stage": m.stage.label(), "algorithm": m.algorithm,
        "task": m.task, "target": m.target, "classes": m.classes,
        "features": m.features.iter().map(|f| json!({"name": f.name, "type": f.dtype, "categories": f.levels})).collect::<Vec<_>>(),
        "example": J::Object(m.features.iter().map(|f| (f.name.clone(), if f.dtype == "number" { json!(0.0) } else { json!(f.levels.first().cloned().unwrap_or_default()) })).collect()),
    })
}

fn predict(m: &RegisteredModel, body: &str) -> Response {
    let v: J = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return err(400, &format!("The body isn't valid JSON: {e}")),
    };
    let rows: Vec<serde_json::Map<String, J>> = match v {
        J::Array(a) => a.into_iter().filter_map(|x| if let J::Object(o) = x { Some(o) } else { None }).collect(),
        J::Object(mut o) => match o.remove("rows").or_else(|| o.remove("instances")).or_else(|| o.remove("inputs")) {
            Some(J::Array(a)) => a.into_iter().filter_map(|x| if let J::Object(o) = x { Some(o) } else { None }).collect(),
            Some(_) => return err(400, "`rows` must be an array of objects."),
            None => vec![o],
        },
        _ => return err(400, "Send an object, an array of objects or {\"rows\": [...]}."),
    };
    if rows.len() > 100_000 {
        return err(413, "At most 100,000 rows per request.");
    }
    let preds: Vec<J> = rows.iter().map(|r| m.model.predict_json(r)).collect();
    json_resp(200, json!({ "model": m.name, "version": m.version, "stage": m.stage.label(), "predictions": preds }))
}

/// Routes one request.
pub fn handle(models: &[RegisteredModel], method: &str, path: &str, body: &str) -> Response {
    let path = path.split('?').next().unwrap_or("/");
    let parts: Vec<String> = path.trim_matches('/').split('/').filter(|s| !s.is_empty()).map(url_decode).collect();
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    let find = |name: &str, version: Option<u32>| -> Option<&RegisteredModel> {
        match version {
            Some(v) => models.iter().find(|m| m.name == name && m.version == v),
            None => crate::records::serving_version(models, name),
        }
    };
    match (method, parts.as_slice()) {
        ("OPTIONS", _) => Response { status: 204, body: String::new(), content_type: "text/plain" },
        ("GET", []) => json_resp(200, json!({"service": "VizCraft model server", "endpoints": ["GET /health", "GET /models", "GET /models/<name>/schema", "POST /models/<name>/predict", "POST /models/<name>/versions/<v>/predict"]})),
        ("GET", ["health"]) => json_resp(200, json!({"status": "ok", "models": summary(models).as_array().map(|a| a.len()).unwrap_or(0)})),
        ("GET", ["models"]) => json_resp(200, summary(models)),
        ("GET", ["models", name]) => match find(name, None) {
            Some(_) => json_resp(200, summary(&models.iter().filter(|m| m.name == *name).cloned().collect::<Vec<_>>()).get(0).cloned().unwrap_or(J::Null)),
            None => err(404, &format!("No model named `{name}`.")),
        },
        ("GET", ["models", name, "schema"]) => match find(name, None) {
            Some(m) => json_resp(200, schema(m)),
            None => err(404, &format!("No model named `{name}`.")),
        },
        ("GET", ["models", name, "versions", v, "schema"]) => match v.trim_start_matches('v').parse().ok().and_then(|v| find(name, Some(v))) {
            Some(m) => json_resp(200, schema(m)),
            None => err(404, &format!("No version {v} of `{name}`.")),
        },
        ("POST", ["models", name, "predict"]) => match find(name, None) {
            Some(m) => predict(m, body),
            None => err(404, &format!("No model named `{name}`.")),
        },
        ("POST", ["models", name, "versions", v, "predict"]) => match v.trim_start_matches('v').parse().ok().and_then(|v| find(name, Some(v))) {
            Some(m) => predict(m, body),
            None => err(404, &format!("No version {v} of `{name}`.")),
        },
        ("GET", ["models", _, "predict"]) => err(405, "Use POST with a JSON body."),
        _ => err(404, "Not found."),
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "Error",
    }
}

fn serve_conn(stream: TcpStream, cache: &Mutex<Cache>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let Ok(clone) = stream.try_clone() else { return };
    let mut reader = BufReader::new(clone);
    let mut stream = stream;
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.is_empty() {
        return;
    }
    let mut it = line.split_whitespace();
    let method = it.next().unwrap_or("").to_string();
    let path = it.next().unwrap_or("/").to_string();
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).is_err() || h == "\r\n" || h == "\n" || h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
        {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let resp = if len > 64 * 1024 * 1024 {
        err(413, "Body too large.")
    } else {
        let mut body = vec![0u8; len];
        if reader.read_exact(&mut body).is_err() {
            err(400, "Incomplete body.")
        } else {
            let body = String::from_utf8_lossy(&body).to_string();
            match cache.lock() {
                Ok(mut c) => {
                    let models = c.models().to_vec();
                    drop(c);
                    handle(&models, &method, &path, &body)
                }
                Err(_) => err(500, "Registry unavailable."),
            }
        }
    };
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}; charset=utf-8\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nConnection: close\r\n\r\n",
        resp.status,
        reason(resp.status),
        resp.content_type,
        resp.body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(resp.body.as_bytes());
    let _ = stream.flush();
}

/// A running server; dropping it (or calling `stop`) shuts it down.
pub struct Handle {
    stop: Arc<AtomicBool>,
    pub addr: String,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Handle {
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
    pub fn running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Binds and serves on a background thread.
pub fn start(registry: Registry, host: &str, port: u16) -> Result<Handle, String> {
    let listener = TcpListener::bind((host, port)).map_err(|e| format!("Couldn't listen on {host}:{port}: {e}"))?;
    let addr = listener.local_addr().map(|a| a.to_string()).unwrap_or_else(|_| format!("{host}:{port}"));
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let cache = Arc::new(Mutex::new(Cache { reg: registry, models: vec![], stamp: None, checked: None }));
    let thread = std::thread::spawn(move || {
        while !stop2.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((s, _)) => {
                    let _ = s.set_nonblocking(false);
                    let c = cache.clone();
                    std::thread::spawn(move || serve_conn(s, &c));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(25)),
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    });
    Ok(Handle { stop, addr, thread: Some(thread) })
}

/// Serves in the current thread until the process ends.
pub fn run_forever(registry: Registry, host: &str, port: u16) -> Result<(), String> {
    let h = start(registry, host, port)?;
    while h.running() {
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

/// A `curl` example for a model's predict endpoint.
pub fn curl_example(base: &str, m: &RegisteredModel) -> String {
    let example: serde_json::Map<String, J> = m.features.iter().map(|f| (f.name.clone(), if f.dtype == "number" { json!(1.0) } else { json!(f.levels.first().cloned().unwrap_or_default()) })).collect();
    let body = serde_json::to_string(&json!({ "rows": [J::Object(example)] })).unwrap_or_default().replace('\'', "'\\''");
    format!("curl -X POST {base}/models/{}/predict \\\n  -H 'Content-Type: application/json' \\\n  -d '{body}'", m.name.replace(' ', "%20"))
}

/// A blocking HTTP GET/POST against a local server (for tests and health checks).
pub fn request(addr: &str, method: &str, path: &str, body: &str) -> Result<(u16, String), String> {
    let mut s = TcpStream::connect(addr).map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let req = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut out = String::new();
    s.read_to_string(&mut out).map_err(|e| e.to_string())?;
    let status = out.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let body = out.split_once("\r\n\r\n").map(|x| x.1.to_string()).unwrap_or_default();
    Ok((status, body))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn serves_predictions() {
        let dir = std::env::temp_dir().join(format!("vc-serve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let reg = Registry::new(&dir);
        let m = reg.save(&crate::tests::model("demo model")).unwrap();
        let mut h = start(reg, "127.0.0.1", 0).unwrap();
        let (st, body) = request(&h.addr, "GET", "/health", "").unwrap();
        assert_eq!(st, 200, "{body}");
        let (st, body) = request(&h.addr, "GET", "/models", "").unwrap();
        assert_eq!(st, 200);
        assert!(body.contains("demo model"));
        let (st, body) = request(&h.addr, "GET", "/models/demo%20model/schema", "").unwrap();
        assert_eq!(st, 200);
        assert!(body.contains("\"x\""));
        let (st, body) = request(&h.addr, "POST", "/models/demo%20model/predict", r#"{"rows":[{"x":9.5,"g":"a"},{"x":1,"g":"b"}]}"#).unwrap();
        assert_eq!(st, 200, "{body}");
        let v: J = serde_json::from_str(&body).unwrap();
        assert_eq!(v["predictions"][0]["prediction"], "yes");
        assert_eq!(v["predictions"][1]["prediction"], "no");
        assert_eq!(v["version"], m.version);
        let (st, _) = request(&h.addr, "POST", "/models/nope/predict", "{}").unwrap();
        assert_eq!(st, 404);
        let (st, _) = request(&h.addr, "POST", "/models/demo%20model/predict", "not json").unwrap();
        assert_eq!(st, 400);
        assert!(curl_example("http://localhost:8090", &m).contains("predict"));
        h.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
