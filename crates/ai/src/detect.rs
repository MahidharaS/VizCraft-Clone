//! Local model server detection: probes the usual ports of Ollama, LM Studio, llama.cpp,
//! vLLM and the OpenJEV helper, in parallel, with short timeouts, and lists their models.

use std::time::Duration;

use serde_json::Value;

use crate::http::get_json;
use crate::provider::ProviderKind;

#[derive(Clone, Debug, PartialEq)]
pub enum ServerKind {
    Llm(ProviderKind),
    OpenJev,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    pub label: &'static str,
    pub kind: ServerKind,
    /// Base URL a provider would use (`…/v1` for OpenAI-compatible servers).
    pub base_url: String,
    /// The URL probed.
    pub probe_url: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Detected {
    pub probe: Probe,
    pub models: Vec<String>,
    /// Extra info (OpenJEV version string, …).
    pub info: String,
}

/// Host:port of Ollama from `OLLAMA_HOST` (default 127.0.0.1:11434).
fn ollama_host() -> String {
    let h = std::env::var("OLLAMA_HOST").unwrap_or_default();
    let h = h.trim().trim_start_matches("http://").trim_start_matches("https://").trim_end_matches('/');
    if h.is_empty() {
        return "127.0.0.1:11434".into();
    }
    let h = h.replace("0.0.0.0", "127.0.0.1");
    if h.contains(':') { h } else { format!("{h}:11434") }
}

/// Probes use 127.0.0.1 (not `localhost`): on macOS `localhost` can resolve to ::1 first while
/// Ollama and llama.cpp listen on IPv4 only, which made running servers look absent.
pub fn default_probes() -> Vec<Probe> {
    let o = ollama_host();
    let mut v = vec![
        Probe { label: "Ollama", kind: ServerKind::Llm(ProviderKind::Ollama), base_url: format!("http://{o}/v1"), probe_url: format!("http://{o}/api/tags") },
        Probe { label: "LM Studio", kind: ServerKind::Llm(ProviderKind::LmStudio), base_url: "http://127.0.0.1:1234/v1".into(), probe_url: "http://127.0.0.1:1234/v1/models".into() },
        Probe { label: "llama.cpp server", kind: ServerKind::Llm(ProviderKind::LlamaCpp), base_url: "http://127.0.0.1:8080/v1".into(), probe_url: "http://127.0.0.1:8080/v1/models".into() },
        Probe { label: "vLLM", kind: ServerKind::Llm(ProviderKind::Vllm), base_url: "http://127.0.0.1:8000/v1".into(), probe_url: "http://127.0.0.1:8000/v1/models".into() },
        Probe { label: "MLX server", kind: ServerKind::Llm(ProviderKind::LlamaCpp), base_url: "http://127.0.0.1:8081/v1".into(), probe_url: "http://127.0.0.1:8081/v1/models".into() },
        Probe { label: "Jan", kind: ServerKind::Llm(ProviderKind::LmStudio), base_url: "http://127.0.0.1:1337/v1".into(), probe_url: "http://127.0.0.1:1337/v1/models".into() },
        Probe { label: "GPT4All", kind: ServerKind::Llm(ProviderKind::LmStudio), base_url: "http://127.0.0.1:4891/v1".into(), probe_url: "http://127.0.0.1:4891/v1/models".into() },
    ];
    for port in [3000, 8765] {
        v.push(Probe { label: "OpenJEV helper", kind: ServerKind::OpenJev, base_url: format!("http://127.0.0.1:{port}"), probe_url: format!("http://127.0.0.1:{port}/v1/version") });
    }
    v
}

/// Model names from `/api/tags` (Ollama) or `/v1/models` (OpenAI-compatible).
pub fn parse_models(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(Value::Array(ms)) = v.get("models") {
        out.extend(ms.iter().filter_map(|m| m.get("name").or_else(|| m.get("model")).and_then(Value::as_str).map(str::to_string)));
    }
    if let Some(Value::Array(ms)) = v.get("data") {
        out.extend(ms.iter().filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string)));
    }
    out
}

/// Probes every server in parallel (each with `timeout`); returns the ones that answered.
pub fn detect(probes: &[Probe], timeout: Duration, token: &str) -> Vec<Detected> {
    let handles: Vec<_> = probes
        .iter()
        .cloned()
        .map(|p| {
            let token = token.to_string();
            std::thread::spawn(move || {
                let headers = if matches!(p.kind, ServerKind::OpenJev) && !token.is_empty() { vec![("Authorization".to_string(), format!("Bearer {token}"))] } else { vec![] };
                let v = get_json(&p.probe_url, &headers, timeout).ok()?;
                let (models, info) = match p.kind {
                    ServerKind::OpenJev => {
                        let m = v.get("model").or_else(|| v.get("served_model")).and_then(Value::as_str).unwrap_or("openjev").to_string();
                        (vec![m.clone()], v.to_string().chars().take(300).collect())
                    }
                    ServerKind::Llm(_) => (parse_models(&v), String::new()),
                };
                Some(Detected { probe: p, models, info })
            })
        })
        .collect();
    let mut found: Vec<Detected> = handles.into_iter().filter_map(|h| h.join().ok().flatten()).collect();
    // The same OpenJEV helper may answer on both ports only once in practice; keep the first.
    found.dedup_by(|a, b| a.probe.kind == b.probe.kind && a.probe.base_url == b.probe.base_url);
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::mock;

    #[test]
    fn detects_mock_servers_and_lists_models() {
        let ollama = mock::serve(vec![(200, "{\"models\":[{\"name\":\"qwen2.5:7b\"},{\"name\":\"llama3.2:3b\"}]}".into())]);
        let lms = mock::serve(vec![(200, "{\"data\":[{\"id\":\"qwen2.5-7b-instruct\"}]}".into())]);
        let jev = mock::serve(vec![(200, "{\"model\":\"openjev T=0.85\"}".into())]);
        let probes = vec![
            Probe { label: "Ollama", kind: ServerKind::Llm(ProviderKind::Ollama), base_url: format!("{}/v1", ollama.url), probe_url: format!("{}/api/tags", ollama.url) },
            Probe { label: "LM Studio", kind: ServerKind::Llm(ProviderKind::LmStudio), base_url: format!("{}/v1", lms.url), probe_url: format!("{}/v1/models", lms.url) },
            Probe { label: "OpenJEV helper", kind: ServerKind::OpenJev, base_url: jev.url.clone(), probe_url: format!("{}/v1/version", jev.url) },
            Probe { label: "Nothing", kind: ServerKind::Llm(ProviderKind::Vllm), base_url: "http://127.0.0.1:9/v1".into(), probe_url: "http://127.0.0.1:9/v1/models".into() },
        ];
        let found = detect(&probes, Duration::from_millis(1500), "");
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].models, vec!["qwen2.5:7b", "llama3.2:3b"]);
        assert_eq!(found[1].models, vec!["qwen2.5-7b-instruct"]);
        assert_eq!(found[2].models, vec!["openjev T=0.85"]);
    }
}
