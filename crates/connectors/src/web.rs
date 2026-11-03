//! `Web.Contents` and `OData.Feed`.
//!
//! Requests go through `HTTPS_PROXY` / `HTTP_PROXY` (and `NO_PROXY`) when set. When
//! `SSL_CERT_FILE` names a PEM bundle, its certificates are the trusted roots (for corporate
//! proxies that re-sign TLS).

use std::time::Duration;

use vizcraft_query::funcs::source::json_value;
use vizcraft_query::{Error, Record, Result, Table, Value};

use crate::Args;

const MAX_BODY: u64 = 512 * 1024 * 1024;

fn agent() -> ureq::Agent {
    let mut b = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(120))).http_status_as_error(false);
    if let Ok(path) = std::env::var("SSL_CERT_FILE")
        && let Ok(pem) = std::fs::read(&path)
    {
        let certs: Vec<ureq::tls::Certificate<'static>> = ureq::tls::parse_pem(&pem)
            .filter_map(|i| match i {
                Ok(ureq::tls::PemItem::Certificate(c)) => Some(c.to_owned()),
                _ => None,
            })
            .collect();
        if !certs.is_empty() {
            b = b.tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::new_with_certs(&certs)).build());
        }
    }
    b.build().into()
}

/// Splits the host's URL string into the URL and its `Name: value` headers.
fn split(url: &str) -> (String, Vec<(String, String)>) {
    let mut parts = url.split('\u{1}');
    let u = parts.next().unwrap_or_default().trim().to_string();
    let headers = parts.filter_map(|h| h.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))).collect();
    (u, headers)
}

/// Fetches a URL (`file://` paths read the file).
pub fn get(url: &str) -> Result<Vec<u8>> {
    get_with(url, &[])
}

fn get_with(url: &str, extra: &[(&str, &str)]) -> Result<Vec<u8>> {
    let (u, headers) = split(url);
    if let Some(p) = u.strip_prefix("file://") {
        return crate::files::read(p);
    }
    if !(u.starts_with("http://") || u.starts_with("https://")) {
        return Err(Error::Eval(format!("DataSource.Error: '{u}' isn't a web address (it should start with https://).")));
    }
    let mut req = agent().get(&u).header("User-Agent", "VizCraft/0.1 (Power Query)");
    for (k, v) in &headers {
        req = req.header(k.as_str(), v.as_str());
    }
    for (k, v) in extra {
        req = req.header(*k, *v);
    }
    let mut resp = req.call().map_err(|e| Error::Eval(format!("DataSource.Error: Web.Contents failed to get contents from '{u}': {e}")))?;
    let status = resp.status();
    let body = resp.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(|e| Error::Eval(format!("DataSource.Error: {u}: {e}")))?;
    if !status.is_success() {
        return Err(Error::Eval(format!("DataSource.Error: Web.Contents failed to get contents from '{u}' ({status}).")));
    }
    Ok(body)
}

/// `OData.Feed(url)`: the `value` array of the feed as a table, following `@odata.nextLink`.
/// A service root lists its entity sets (`Name`, `Kind`, `URL`).
pub fn odata(a: &Args) -> Result<Table> {
    let Some(start) = a.pos(0) else { return crate::err("OData.Feed needs a URL.") };
    let mut url = start.to_string();
    let mut recs: Vec<Record> = Vec::new();
    for _ in 0..200 {
        let body = get_with(&url, &[("Accept", "application/json;odata.metadata=minimal")])?;
        let j: serde_json::Value = serde_json::from_slice(&body).map_err(|e| Error::Eval(format!("DataFormat.Error: The feed isn't OData JSON ({e}).")))?;
        let items = j.get("value").and_then(|v| v.as_array()).cloned().or_else(|| j.get("d").and_then(|d| d.get("results")).and_then(|v| v.as_array()).cloned());
        let Some(items) = items else { return crate::err("DataFormat.Error: The feed has no `value` array.") };
        for it in &items {
            if let Value::Record(r) = json_value(it) {
                let mut r = (*r).clone();
                // Drop OData annotations.
                let keep: Vec<usize> = (0..r.names.len()).filter(|i| !r.names.get(*i).is_some_and(|n| n.contains("@odata") || n == "__metadata")).collect();
                r.names = keep.iter().filter_map(|i| r.names.get(*i).cloned()).collect();
                r.values = keep.iter().filter_map(|i| r.values.get(*i).cloned()).collect();
                recs.push(r);
            }
        }
        match j.get("@odata.nextLink").or_else(|| j.get("odata.nextLink")).and_then(|v| v.as_str()) {
            Some(next) => url = next.to_string(),
            None => break,
        }
    }
    let mut t = crate::table_from_records(&recs);
    crate::infer_types(&mut t);
    Ok(t)
}
