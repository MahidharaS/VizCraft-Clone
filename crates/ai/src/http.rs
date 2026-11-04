//! Minimal JSON-over-HTTP on top of `ureq` (rustls). Every call is blocking: callers run it on a
//! background thread (the app never calls this on the UI thread).

use std::time::Duration;

use serde_json::Value;

use crate::AiError;

/// True for loopback / LAN-local hosts: those requests never go through an HTTP(S) proxy.
pub fn is_local(url: &str) -> bool {
    let host = url.split("://").nth(1).unwrap_or(url).split(['/', ':']).next().unwrap_or("");
    host == "localhost" || host == "127.0.0.1" || host == "::1" || host == "[::1]" || host == "0.0.0.0" || host.ends_with(".local")
}

fn agent(url: &str, timeout: Duration) -> ureq::Agent {
    let proxy = if is_local(url) { None } else { ureq::Proxy::try_from_env() };
    let cfg = ureq::Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false).proxy(proxy).build();
    ureq::Agent::new_with_config(cfg)
}

/// Posts a JSON body and parses the JSON answer. Non-2xx statuses become [`AiError::Http`] with
/// the provider's error message when it has one.
pub fn post_json(url: &str, headers: &[(String, String)], body: &Value, timeout: Duration) -> Result<Value, AiError> {
    let mut req = agent(url, timeout).post(url).header("Content-Type", "application/json");
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send(body.to_string()).map_err(|e| AiError::Network(e.to_string()))?;
    read(resp)
}

pub fn get_json(url: &str, headers: &[(String, String)], timeout: Duration) -> Result<Value, AiError> {
    let mut req = agent(url, timeout).get(url);
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.call().map_err(|e| AiError::Network(e.to_string()))?;
    read(resp)
}

fn read(mut resp: ureq::http::Response<ureq::Body>) -> Result<Value, AiError> {
    let status = resp.status().as_u16();
    let text = resp.body_mut().with_config().limit(32 * 1024 * 1024).read_to_string().map_err(|e| AiError::Network(e.to_string()))?;
    if !(200..300).contains(&status) {
        let msg = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message").or_else(|| v.get("error")).or_else(|| v.get("message")).or_else(|| v.get("detail")).map(|m| match m {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
            })
            .unwrap_or_else(|| text.chars().take(400).collect());
        return Err(AiError::Http { status, message: msg });
    }
    serde_json::from_str(&text).map_err(|e| AiError::BadResponse(format!("not JSON ({e}): {}", text.chars().take(200).collect::<String>())))
}

#[cfg(test)]
pub mod mock {
    //! A tiny HTTP server for tests: answers each request with the next canned response and
    //! records what it received.
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Debug, Default)]
    pub struct Received {
        pub method: String,
        pub path: String,
        pub headers: Vec<(String, String)>,
        pub body: String,
    }

    pub struct Mock {
        pub url: String,
        pub received: Arc<Mutex<Vec<Received>>>,
    }

    /// Starts a server that answers with `responses` in order (status, body), then 404s.
    pub fn serve(responses: Vec<(u16, String)>) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://127.0.0.1:{}", listener.local_addr().expect("addr").port());
        let received = Arc::new(Mutex::new(Vec::new()));
        let rec = received.clone();
        std::thread::spawn(move || {
            let mut queue = responses.into_iter();
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let path = parts.next().unwrap_or("").to_string();
                let mut headers = Vec::new();
                let mut len = 0usize;
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).is_err() || h.trim().is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.trim().split_once(':') {
                        if k.eq_ignore_ascii_case("content-length") {
                            len = v.trim().parse().unwrap_or(0);
                        }
                        headers.push((k.trim().to_lowercase(), v.trim().to_string()));
                    }
                }
                let mut body = vec![0u8; len];
                let _ = reader.read_exact(&mut body);
                rec.lock().expect("lock").push(Received { method, path, headers, body: String::from_utf8_lossy(&body).to_string() });
                let (status, text) = queue.next().unwrap_or((404, "{\"error\":{\"message\":\"no more responses\"}}".into()));
                let resp = format!("HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len());
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        });
        Mock { url, received }
    }
}
