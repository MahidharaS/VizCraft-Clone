//! Where published reports live: `<data>/<workspace>/<report>/meta.json` and `files/…`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Meta {
    pub workspace: String,
    pub workspace_slug: String,
    pub name: String,
    pub slug: String,
    /// RFC 3339 (UTC).
    pub published: String,
    pub pages: Vec<String>,
    pub interactive: bool,
    pub mobile: bool,
    pub bytes: u64,
    pub url: String,
}

/// Lowercase letters, digits and dashes.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "report".into() } else { out.chars().take(80).collect() }
}

pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn new(root: PathBuf) -> Result<Store, String> {
        std::fs::create_dir_all(&root).map_err(|e| format!("Couldn't create the data folder {}: {e}", root.display()))?;
        Ok(Store { root })
    }

    pub fn report_dir(&self, ws: &str, slug: &str) -> Option<PathBuf> {
        if !valid(ws) || !valid(slug) {
            return None;
        }
        Some(self.root.join(ws).join(slug))
    }

    /// Every report, newest first.
    pub fn list(&self) -> Vec<Meta> {
        let mut out = Vec::new();
        let Ok(wss) = std::fs::read_dir(&self.root) else { return out };
        for ws in wss.flatten() {
            let Ok(reports) = std::fs::read_dir(ws.path()) else { continue };
            for r in reports.flatten() {
                if let Ok(text) = std::fs::read_to_string(r.path().join("meta.json"))
                    && let Ok(m) = serde_json::from_str::<Meta>(&text)
                {
                    out.push(m);
                }
            }
        }
        out.sort_by(|a, b| b.published.cmp(&a.published));
        out
    }

    pub fn get(&self, ws: &str, slug: &str) -> Option<Meta> {
        let dir = self.report_dir(ws, slug)?;
        serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).ok()?).ok()
    }

    /// Workspaces (display name, slug, report count).
    pub fn workspaces(&self) -> Vec<(String, String, usize)> {
        let mut out: Vec<(String, String, usize)> = Vec::new();
        for m in self.list() {
            match out.iter_mut().find(|w| w.1 == m.workspace_slug) {
                Some(w) => w.2 += 1,
                None => out.push((m.workspace.clone(), m.workspace_slug.clone(), 1)),
            }
        }
        out.sort_by_key(|a| a.0.to_lowercase());
        out
    }

    /// Stores a bundle (replacing a report with the same name in the workspace).
    pub fn put(&self, workspace: &str, name: &str, files: &[(String, Vec<u8>)]) -> Result<Meta, String> {
        if !files.iter().any(|(n, _)| n == "index.html") {
            return Err("The bundle has no index.html.".into());
        }
        let ws_slug = slug(workspace);
        let r_slug = slug(name);
        let ws_dir = self.root.join(&ws_slug);
        std::fs::create_dir_all(&ws_dir).map_err(|e| e.to_string())?;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let tmp = ws_dir.join(format!(".{r_slug}.incoming-{nanos}"));
        vizardcraft_publish::net::write_folder(&files.to_vec(), &tmp.join("files"))?;
        let report: Option<vizardcraft_publish::ReportData> = files.iter().find(|(n, _)| n == "report.json").and_then(|(_, b)| serde_json::from_slice(b).ok());
        let meta = Meta {
            workspace: workspace.trim().to_string(),
            workspace_slug: ws_slug.clone(),
            name: name.trim().to_string(),
            slug: r_slug.clone(),
            published: vizardcraft_publish::rfc3339(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)),
            pages: report.as_ref().map(|r| r.pages.iter().filter(|p| !p.hidden).map(|p| p.name.clone()).collect()).unwrap_or_default(),
            interactive: report.as_ref().is_some_and(|r| r.interactive),
            mobile: report.as_ref().is_some_and(|r| r.include_mobile),
            bytes: files.iter().map(|(_, b)| b.len() as u64).sum(),
            url: format!("/r/{ws_slug}/{r_slug}"),
        };
        std::fs::write(tmp.join("meta.json"), serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let dest = ws_dir.join(&r_slug);
        let old = ws_dir.join(format!(".{r_slug}.old-{nanos}"));
        if dest.exists() {
            std::fs::rename(&dest, &old).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(&old);
        Ok(meta)
    }

    pub fn delete(&self, ws: &str, slug: &str) -> Result<(), String> {
        let dir = self.report_dir(ws, slug).ok_or("Not found.")?;
        if !dir.join("meta.json").exists() {
            return Err("Not found.".into());
        }
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
        // Drop an empty workspace.
        if let Some(parent) = dir.parent()
            && std::fs::read_dir(parent).map(|mut d| d.next().is_none()).unwrap_or(false)
        {
            let _ = std::fs::remove_dir(parent);
        }
        Ok(())
    }

    /// A file of a report's bundle (path checked to stay inside it).
    pub fn file(&self, ws: &str, slug: &str, rel: &str) -> Option<PathBuf> {
        let dir = self.report_dir(ws, slug)?.join("files");
        let rel = if rel.is_empty() { "index.html" } else { rel };
        let p = Path::new(rel);
        if p.is_absolute() || p.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
            return None;
        }
        let full = dir.join(p);
        full.is_file().then_some(full)
    }
}

fn valid(s: &str) -> bool {
    !s.is_empty() && !s.starts_with('.') && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("Retail Sales (sample)"), "retail-sales-sample");
        assert_eq!(slug("  "), "report");
        assert_eq!(slug("../etc"), "etc");
    }

    #[test]
    fn put_list_delete() {
        let dir = std::env::temp_dir().join(format!("vc-store-test-{}", std::process::id()));
        let s = Store::new(dir.clone()).unwrap();
        let files = vec![("index.html".to_string(), b"<html>".to_vec()), ("report.json".to_string(), b"{}".to_vec())];
        let m = s.put("Sales Team", "Q3 Review", &files).unwrap();
        assert_eq!(m.url, "/r/sales-team/q3-review");
        assert_eq!(s.list().len(), 1);
        assert!(s.file("sales-team", "q3-review", "").is_some());
        assert!(s.file("sales-team", "q3-review", "../meta.json").is_none());
        s.put("Sales Team", "Q3 Review", &files).unwrap();
        assert_eq!(s.list().len(), 1);
        s.delete("sales-team", "q3-review").unwrap();
        assert!(s.list().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
