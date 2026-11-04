//! Sending bundles somewhere: a folder, a zip, a VizardCraft Service (HTTP), GitHub Pages (git).
//! Everything here blocks: call it from a worker thread in a UI.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::Files;

/// Writes a bundle into a folder (created when missing).
pub fn write_folder(files: &Files, dir: &Path) -> Result<(), String> {
    for (name, bytes) in files {
        let rel = Path::new(name);
        if rel.is_absolute() || rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(format!("Refusing to write {name} outside the folder."));
        }
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Couldn't create {}: {e}", parent.display()))?;
        }
        std::fs::write(&path, bytes).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
    }
    Ok(())
}

/// A bundle as a zip archive.
pub fn zip_files(files: &Files) -> Result<Vec<u8>, String> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in files {
            z.start_file(name.as_str(), opts).map_err(|e| e.to_string())?;
            z.write_all(bytes).map_err(|e| e.to_string())?;
        }
        z.finish().map_err(|e| e.to_string())?;
    }
    Ok(buf.into_inner())
}

/// Reads a zip archive into files (paths that would leave the folder are skipped).
pub fn unzip_files(bytes: &[u8]) -> Result<Files, String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("Not a zip archive: {e}"))?;
    let mut out = Vec::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| e.to_string())?;
        if f.is_dir() {
            continue;
        }
        let Some(name) = f.enclosed_name().map(|p| p.to_string_lossy().replace('\\', "/")) else { continue };
        let mut data = Vec::new();
        f.read_to_end(&mut data).map_err(|e| e.to_string())?;
        out.push((name, data));
    }
    Ok(out)
}

/// Percent-encodes a query-string value.
pub fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(std::time::Duration::from_secs(10)).timeout(std::time::Duration::from_secs(300)).build()
}

fn http_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            let msg = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|j| j.get("error").and_then(|e| e.as_str()).map(String::from)).unwrap_or(body);
            match code {
                401 | 403 => format!("The service refused the token ({code}). {msg}"),
                _ => format!("The service answered {code}. {msg}"),
            }
        }
        ureq::Error::Transport(t) => format!("Couldn't reach the service: {t}"),
    }
}

/// Checks a VizardCraft Service: reachable, and whether the token may publish.
pub fn service_ping(url: &str, token: &str) -> Result<String, String> {
    let base = url.trim_end_matches('/');
    let mut req = agent().get(&format!("{base}/api/ping"));
    if !token.is_empty() {
        req = req.set("Authorization", &format!("Bearer {token}"));
    }
    let resp = req.call().map_err(http_err)?;
    let j: serde_json::Value = resp.into_json().map_err(|e| format!("That doesn't look like a VizardCraft Service: {e}"))?;
    let name = j.get("name").and_then(|v| v.as_str()).unwrap_or("VizardCraft Service");
    let reports = j.get("reports").and_then(|v| v.as_u64()).unwrap_or(0);
    match j.get("canPublish").and_then(|v| v.as_bool()) {
        Some(true) => Ok(format!("Connected to {name} ({reports} reports). The token can publish.")),
        Some(false) => Err(format!("Connected to {name}, but this token can't publish.")),
        None => Ok(format!("Connected to {name}.")),
    }
}

/// Publishes a bundle to a VizardCraft Service; returns the report's URL.
pub fn publish_to_service(url: &str, workspace: &str, name: &str, token: &str, files: &Files) -> Result<String, String> {
    let base = url.trim_end_matches('/');
    if !(base.starts_with("http://") || base.starts_with("https://")) {
        return Err("The service URL must start with http:// or https://".into());
    }
    let body = zip_files(files)?;
    let endpoint = format!("{base}/api/publish?workspace={}&name={}", url_encode(workspace), url_encode(name));
    let mut req = agent().post(&endpoint).set("Content-Type", "application/zip");
    if !token.is_empty() {
        req = req.set("Authorization", &format!("Bearer {token}"));
    }
    let resp = req.send_bytes(&body).map_err(http_err)?;
    let j: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
    let path = j.get("url").and_then(|v| v.as_str()).unwrap_or("/");
    Ok(if path.starts_with("http") { path.to_string() } else { format!("{base}{path}") })
}

// ---------------------------------------------------------------------------------------------
// GitHub Pages
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct GitHubTarget {
    /// `owner/repo`.
    pub repo: String,
    /// Default `gh-pages`.
    pub branch: String,
    /// Subfolder in the branch ("" = the root).
    pub path: String,
    /// Personal access token (contents: write; pages: write to enable Pages).
    pub token: String,
    /// Custom domain (writes `CNAME`).
    pub domain: String,
}

impl GitHubTarget {
    pub fn owner_repo(&self) -> Result<(String, String), String> {
        let r = self.repo.trim().trim_start_matches("https://github.com/").trim_end_matches(".git").trim_matches('/');
        match r.split_once('/') {
            Some((o, n)) if !o.is_empty() && !n.is_empty() && !n.contains('/') && valid_name(o) && valid_name(n) => Ok((o.to_string(), n.to_string())),
            _ => Err("The repository must look like owner/repo.".into()),
        }
    }
    fn branch(&self) -> String {
        let b = self.branch.trim();
        if b.is_empty() { "gh-pages".into() } else { b.to_string() }
    }
    fn sub(&self) -> String {
        self.path.trim().trim_matches('/').to_string()
    }
    /// Where the site will be.
    pub fn site_url(&self) -> String {
        let (owner, repo) = self.owner_repo().unwrap_or_default();
        let sub = self.sub();
        let tail = if sub.is_empty() { String::new() } else { format!("{sub}/") };
        let d = self.domain.trim();
        if !d.is_empty() {
            return format!("https://{d}/{tail}");
        }
        if repo.eq_ignore_ascii_case(&format!("{owner}.github.io")) {
            format!("https://{}.github.io/{tail}", owner.to_lowercase())
        } else {
            format!("https://{}.github.io/{repo}/{tail}", owner.to_lowercase())
        }
    }
    /// DNS instructions for the custom domain.
    pub fn dns_help(&self) -> Option<String> {
        let d = self.domain.trim();
        if d.is_empty() {
            return None;
        }
        let (owner, _) = self.owner_repo().unwrap_or_default();
        let apex = d.matches('.').count() == 1;
        Some(if apex {
            format!(
                "At your DNS provider, add A records for {d} pointing to 185.199.108.153, 185.199.109.153, 185.199.110.153 and 185.199.111.153 (or an ALIAS/ANAME record to {}.github.io). Then tick \"Enforce HTTPS\" in the repository's Settings → Pages.",
                owner.to_lowercase()
            )
        } else {
            format!("At your DNS provider, add a CNAME record: {d} → {}.github.io. Then tick \"Enforce HTTPS\" in the repository's Settings → Pages.", owner.to_lowercase())
        })
    }
}

fn valid_name(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    std::env::temp_dir().join(format!("vizardcraft-{tag}-{}-{nanos}", std::process::id()))
}

fn git(dir: &Path, args: &[&str], token: &str) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("Couldn't run git ({e}). Install Git and try again."))?;
    let mask = |s: &str| if token.is_empty() { s.to_string() } else { s.replace(token, "***") };
    if out.status.success() {
        Ok(mask(&String::from_utf8_lossy(&out.stdout)))
    } else {
        Err(mask(&format!("git {} failed: {}", args.first().copied().unwrap_or(""), String::from_utf8_lossy(&out.stderr).trim())))
    }
}

/// Publishes a bundle to GitHub Pages: commits it to the branch (keeping what else is there)
/// and pushes. Returns the site URL.
pub fn publish_github_pages(t: &GitHubTarget, files: &Files, message: &str, progress: &dyn Fn(&str)) -> Result<String, String> {
    let (owner, repo) = t.owner_repo()?;
    if t.token.trim().is_empty() {
        return Err("A personal access token is needed to push to GitHub.".into());
    }
    let branch = t.branch();
    if !branch.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')) {
        return Err("That branch name isn't valid.".into());
    }
    let token = t.token.trim();
    let remote = format!("https://x-access-token:{token}@github.com/{owner}/{repo}.git");
    let dir = temp_dir("gh");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        progress("Preparing the branch…");
        git(&dir, &["init", "-q"], token)?;
        git(&dir, &["remote", "add", "origin", &remote], token)?;
        let exists = git(&dir, &["fetch", "-q", "--depth", "1", "origin", &branch], token).is_ok();
        if exists {
            git(&dir, &["checkout", "-q", "-B", &branch, "FETCH_HEAD"], token)?;
        } else {
            git(&dir, &["checkout", "-q", "--orphan", &branch], token)?;
        }
        let sub = t.sub();
        let target = if sub.is_empty() { dir.clone() } else { dir.join(&sub) };
        // Replace the previous report in the target folder (keep .git, CNAME and other folders'
        // reports when publishing into a subfolder).
        if target.exists() {
            for entry in std::fs::read_dir(&target).map_err(|e| e.to_string())?.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name == ".git" || name == "CNAME" || (sub.is_empty() && entry.path().is_dir()) {
                    continue;
                }
                let p = entry.path();
                let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
            }
        }
        progress("Copying the report…");
        write_folder(files, &target)?;
        std::fs::write(dir.join(".nojekyll"), b"").map_err(|e| e.to_string())?;
        let domain = t.domain.trim();
        if !domain.is_empty() {
            std::fs::write(dir.join("CNAME"), format!("{domain}\n")).map_err(|e| e.to_string())?;
        }
        git(&dir, &["add", "-A"], token)?;
        let status = git(&dir, &["status", "--porcelain"], token)?;
        if !status.trim().is_empty() {
            git(&dir, &["-c", "user.name=VizardCraft", "-c", "user.email=vizardcraft@users.noreply.github.com", "commit", "-q", "-m", message], token)?;
            progress("Pushing to GitHub…");
            git(&dir, &["push", "-q", "origin", &format!("HEAD:refs/heads/{branch}")], token)?;
        }
        progress("Turning on GitHub Pages…");
        enable_pages(&owner, &repo, &branch, token, domain);
        Ok(t.site_url())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// Best effort: enables Pages for the branch and sets the custom domain (needs the token's
/// "pages" permission; ignored when it lacks it or Pages is already on).
fn enable_pages(owner: &str, repo: &str, branch: &str, token: &str, domain: &str) {
    let api = format!("https://api.github.com/repos/{owner}/{repo}/pages");
    let a = agent();
    let auth = format!("Bearer {token}");
    let _ = a
        .post(&api)
        .set("Authorization", &auth)
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", "VizardCraft")
        .send_json(serde_json::json!({"source": {"branch": branch, "path": "/"}}));
    if !domain.is_empty() {
        let _ = a.put(&api).set("Authorization", &auth).set("Accept", "application/vnd.github+json").set("User-Agent", "VizardCraft").send_json(serde_json::json!({"cname": domain}));
    }
}

/// Opens a file or URL with the system's default app.
pub fn open_in_browser(target: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let r = Command::new("open").arg(target).spawn();
    #[cfg(target_os = "windows")]
    let r = Command::new("cmd").args(["/C", "start", "", target]).spawn();
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let r = Command::new("xdg-open").arg(target).spawn();
    r.map(|_| ()).map_err(|e| format!("Couldn't open {target}: {e}"))
}

/// Writes a single-file report to a temp folder and returns its path (Preview in browser).
pub fn preview_file(html: &str, name: &str) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("vizardcraft-preview");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let path = dir.join(format!("{}.html", if safe.is_empty() { "report".into() } else { safe }));
    std::fs::write(&path, html).map_err(|e| e.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn zip_roundtrip() {
        let files: Files = vec![("index.html".into(), b"<html>".to_vec()), ("a/b.txt".into(), vec![1, 2, 3])];
        let z = zip_files(&files).unwrap();
        assert_eq!(unzip_files(&z).unwrap(), files);
    }

    #[test]
    fn github_urls() {
        let t = GitHubTarget { repo: "Acme/reports".into(), ..GitHubTarget::default() };
        assert_eq!(t.site_url(), "https://acme.github.io/reports/");
        let t = GitHubTarget { repo: "acme/acme.github.io".into(), path: "sales".into(), ..GitHubTarget::default() };
        assert_eq!(t.site_url(), "https://acme.github.io/sales/");
        let t = GitHubTarget { repo: "acme/r".into(), domain: "reports.acme.com".into(), ..GitHubTarget::default() };
        assert_eq!(t.site_url(), "https://reports.acme.com/");
        assert!(t.dns_help().unwrap().contains("CNAME record: reports.acme.com → acme.github.io"));
        assert!(GitHubTarget { repo: "bad".into(), ..GitHubTarget::default() }.owner_repo().is_err());
    }
}
