//! Models installed on this computer, found on disk (not only servers that are running):
//! Ollama's model store, LM Studio's models, loose GGUF files (llama.cpp), and the Hugging Face
//! cache (including MLX models). Also finds the runtimes' command-line tools, even when the app
//! was started from the Dock/Finder without the terminal's PATH, and can start their servers.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A model found on disk.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    /// "Ollama", "LM Studio", "llama.cpp (GGUF)", "Hugging Face", "MLX".
    pub runtime: &'static str,
    /// The name to ask the server for (`qwen2.5:7b`), or a display name.
    pub name: String,
    pub path: Option<PathBuf>,
    pub size_bytes: u64,
}

impl Installed {
    pub fn size_label(&self) -> String {
        let b = self.size_bytes as f64;
        if b >= 1e9 {
            format!("{:.1} GB", b / 1e9)
        } else if b >= 1e6 {
            format!("{:.0} MB", b / 1e6)
        } else if b > 0.0 {
            format!("{:.0} KB", b / 1e3)
        } else {
            String::new()
        }
    }
}

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

/// Directories searched for command-line tools besides PATH (GUI apps on macOS don't inherit the
/// shell's PATH, so `ollama` from Homebrew isn't visible to them otherwise).
fn extra_bin_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin", "/opt/local/bin", "/snap/bin", "/Applications/Ollama.app/Contents/Resources", "/Applications/LM Studio.app/Contents/Resources/app/.webpack/lms"].iter().map(PathBuf::from).collect();
    if let Some(h) = home() {
        for d in [".local/bin", ".lmstudio/bin", ".cache/lm-studio/bin", ".cargo/bin", "bin", "miniconda3/bin", "anaconda3/bin", ".pyenv/shims", "llama.cpp/build/bin", "llama.cpp"] {
            v.push(h.join(d));
        }
        v.push(h.join("AppData/Local/Programs/Ollama"));
    }
    if let Some(l) = std::env::var_os("LOCALAPPDATA") {
        v.push(PathBuf::from(l).join("Programs/Ollama"));
    }
    v
}

/// Finds a command-line tool on PATH or in the usual install locations.
pub fn find_bin(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(extra_bin_dirs());
    dirs.into_iter().map(|d| d.join(&exe)).find(|p| p.is_file())
}

/// A PATH that includes the extra directories (for child processes).
pub fn full_path() -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(extra_bin_dirs());
    std::env::join_paths(dirs).unwrap_or_default()
}

fn dir_size(p: &Path, depth: usize) -> u64 {
    if p.is_file() {
        return std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    }
    if depth == 0 {
        return 0;
    }
    std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| dir_size(&e.path(), depth - 1)).sum()).unwrap_or(0)
}

fn files_with_ext(root: &Path, ext: &str, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(root) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let is_dir = e.file_type().map(|t| t.is_dir() || t.is_symlink() && p.is_dir()).unwrap_or(false);
        if is_dir {
            if depth > 0 && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
                files_with_ext(&p, ext, depth - 1, out);
            }
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)) {
            out.push(p);
        }
    }
}

/// Ollama's model store: `manifests/<registry>/<namespace>/<model>/<tag>`.
pub fn ollama_models(store: &Path) -> Vec<Installed> {
    let mut out = vec![];
    let manifests = store.join("manifests");
    let Ok(regs) = std::fs::read_dir(&manifests) else { return out };
    for reg in regs.flatten() {
        let registry = reg.file_name().to_string_lossy().to_string();
        for ns in std::fs::read_dir(reg.path()).into_iter().flatten().flatten() {
            let namespace = ns.file_name().to_string_lossy().to_string();
            for model in std::fs::read_dir(ns.path()).into_iter().flatten().flatten() {
                let m = model.file_name().to_string_lossy().to_string();
                for tag in std::fs::read_dir(model.path()).into_iter().flatten().flatten() {
                    let t = tag.file_name().to_string_lossy().to_string();
                    let base = if registry == "registry.ollama.ai" && namespace == "library" {
                        m.clone()
                    } else if registry == "registry.ollama.ai" {
                        format!("{namespace}/{m}")
                    } else {
                        format!("{registry}/{namespace}/{m}")
                    };
                    // Model size: sum of the layer blobs the manifest names.
                    let size = std::fs::read_to_string(tag.path())
                        .ok()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                        .and_then(|v| v.get("layers").and_then(|l| l.as_array()).map(|ls| ls.iter().filter_map(|l| l.get("size").and_then(|x| x.as_u64())).sum()))
                        .unwrap_or(0);
                    out.push(Installed { runtime: "Ollama", name: format!("{base}:{t}"), path: Some(tag.path()), size_bytes: size });
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// `ollama list` output → model names (when the CLI is available).
pub fn parse_ollama_list(s: &str) -> Vec<String> {
    s.lines().skip(1).filter_map(|l| l.split_whitespace().next()).filter(|n| !n.is_empty()).map(String::from).collect()
}

/// Hugging Face cache: `hub/models--org--name`.
pub fn hf_models(hub: &Path) -> Vec<Installed> {
    let mut out = vec![];
    let Ok(rd) = std::fs::read_dir(hub) else { return out };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        let Some(rest) = n.strip_prefix("models--") else { continue };
        let name = rest.replacen("--", "/", 1);
        let mut ggufs = vec![];
        files_with_ext(&e.path(), "gguf", 4, &mut ggufs);
        let runtime = if name.to_lowercase().starts_with("mlx-community/") || name.to_lowercase().contains("-mlx") {
            "MLX"
        } else if !ggufs.is_empty() {
            "llama.cpp (GGUF)"
        } else {
            "Hugging Face"
        };
        if !ggufs.is_empty() {
            for g in ggufs {
                let size = std::fs::metadata(&g).map(|m| m.len()).unwrap_or(0);
                out.push(Installed { runtime, name: format!("{name}/{}", g.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()), path: Some(g), size_bytes: size });
            }
        } else {
            out.push(Installed { runtime, name, path: Some(e.path()), size_bytes: dir_size(&e.path().join("blobs"), 1) });
        }
    }
    out
}

/// Everything installed on this computer (fast: a few directory listings).
pub fn scan() -> Vec<Installed> {
    let mut out = vec![];
    let Some(h) = home() else { return out };
    // Ollama.
    let store = std::env::var_os("OLLAMA_MODELS").map(PathBuf::from).unwrap_or_else(|| h.join(".ollama/models"));
    let mut ollama = ollama_models(&store);
    if ollama.is_empty() {
        for alt in ["/usr/share/ollama/.ollama/models", "/var/lib/ollama/.ollama/models"] {
            ollama.extend(ollama_models(Path::new(alt)));
        }
    }
    if ollama.is_empty()
        && let Some(bin) = find_bin("ollama")
        && let Ok(o) = Command::new(bin).arg("list").env("PATH", full_path()).stdin(Stdio::null()).stderr(Stdio::null()).output()
    {
        ollama.extend(parse_ollama_list(&String::from_utf8_lossy(&o.stdout)).into_iter().map(|n| Installed { runtime: "Ollama", name: n, path: None, size_bytes: 0 }));
    }
    out.extend(ollama);
    // LM Studio.
    for d in [h.join(".lmstudio/models"), h.join(".cache/lm-studio/models")] {
        let mut g = vec![];
        files_with_ext(&d, "gguf", 4, &mut g);
        for p in g {
            let rel = p.strip_prefix(&d).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            out.push(Installed { runtime: "LM Studio", name: rel, path: Some(p), size_bytes: size });
        }
        // MLX models in LM Studio are folders with config.json + *.safetensors.
        for pubdir in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            for m in std::fs::read_dir(pubdir.path()).into_iter().flatten().flatten() {
                let p = m.path();
                if p.join("config.json").is_file() && std::fs::read_dir(&p).map(|rd| rd.flatten().any(|f| f.path().extension().is_some_and(|e| e == "safetensors"))).unwrap_or(false) {
                    out.push(Installed { runtime: "LM Studio", name: format!("{}/{}", pubdir.file_name().to_string_lossy(), m.file_name().to_string_lossy()), path: Some(p.clone()), size_bytes: dir_size(&p, 1) });
                }
            }
        }
    }
    // Hugging Face cache (transformers, MLX, GGUF downloads).
    let hf = std::env::var_os("HF_HOME").map(|p| PathBuf::from(p).join("hub")).unwrap_or_else(|| h.join(".cache/huggingface/hub"));
    out.extend(hf_models(&hf));
    // Loose GGUF files in the usual places.
    for d in ["models", "Models", "llama.cpp/models", ".cache/llama.cpp", "Downloads", ".ollama-gguf"] {
        let mut g = vec![];
        files_with_ext(&h.join(d), "gguf", if d == "Downloads" { 1 } else { 3 }, &mut g);
        for p in g {
            if out.iter().any(|i| i.path.as_deref() == Some(p.as_path())) {
                continue;
            }
            let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            out.push(Installed { runtime: "llama.cpp (GGUF)", name: p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(), path: Some(p), size_bytes: size });
        }
    }
    out
}

/// The command that serves a model, for display ("ollama serve", "llama-server -m …").
pub fn serve_command(i: &Installed) -> Option<String> {
    match i.runtime {
        "Ollama" => Some("ollama serve".into()),
        "LM Studio" => Some(format!("lms server start && lms load \"{}\"", i.name)),
        "llama.cpp (GGUF)" => i.path.as_ref().map(|p| format!("llama-server -m \"{}\" --port 8080 --jinja", p.display())),
        "MLX" => Some(format!("mlx_lm.server --model {} --port 8081", i.name)),
        _ => None,
    }
}

/// Starts the server for an installed model in the background (detached; it keeps running after
/// VizCraft quits). Returns the OpenAI-compatible base URL to use.
pub fn start_server(i: &Installed) -> Result<String, String> {
    let spawn = |bin: &str, args: &[&str]| -> Result<(), String> {
        let path = find_bin(bin).ok_or_else(|| format!("`{bin}` isn't installed (or not on PATH)."))?;
        Command::new(path).args(args).env("PATH", full_path()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ()).map_err(|e| format!("Couldn't start {bin}: {e}"))
    };
    match i.runtime {
        "Ollama" => {
            // On macOS the Ollama app can serve too.
            if find_bin("ollama").is_none() && Path::new("/Applications/Ollama.app").exists() {
                Command::new("open").args(["-a", "Ollama"]).spawn().map_err(|e| e.to_string())?;
            } else {
                spawn("ollama", &["serve"])?;
            }
            Ok("http://127.0.0.1:11434/v1".into())
        }
        "LM Studio" => {
            spawn("lms", &["server", "start"])?;
            let _ = spawn("lms", &["load", &i.name, "-y"]);
            Ok("http://127.0.0.1:1234/v1".into())
        }
        "llama.cpp (GGUF)" => {
            let p = i.path.as_ref().ok_or("No model file")?.to_string_lossy().to_string();
            spawn("llama-server", &["-m", &p, "--port", "8080", "--jinja"])?;
            Ok("http://127.0.0.1:8080/v1".into())
        }
        "MLX" => {
            spawn("mlx_lm.server", &["--model", &i.name, "--port", "8081"])?;
            Ok("http://127.0.0.1:8081/v1".into())
        }
        other => Err(format!("{other} models are used through a server (Ollama, LM Studio, llama.cpp or MLX).")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_ollama_hf_and_gguf_models_on_disk() {
        let root = std::env::temp_dir().join(format!("vc-installed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let man = root.join("ollama/manifests/registry.ollama.ai/library/qwen2.5");
        std::fs::create_dir_all(&man).unwrap();
        std::fs::write(man.join("7b"), r#"{"layers":[{"size":4000000000},{"size":12}]}"#).unwrap();
        let user = root.join("ollama/manifests/registry.ollama.ai/someone/tiny");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(user.join("latest"), "{}").unwrap();
        let found = ollama_models(&root.join("ollama"));
        let names: Vec<&str> = found.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["qwen2.5:7b", "someone/tiny:latest"]);
        assert_eq!(found[0].size_label(), "4.0 GB");

        let hub = root.join("hf");
        std::fs::create_dir_all(hub.join("models--mlx-community--Qwen2.5-7B-4bit/blobs")).unwrap();
        let snap = hub.join("models--bartowski--Llama-3.2-3B-GGUF/snapshots/abc");
        std::fs::create_dir_all(&snap).unwrap();
        std::fs::write(snap.join("Llama-3.2-3B-Q4_K_M.gguf"), b"gguf").unwrap();
        let hf = hf_models(&hub);
        assert!(hf.iter().any(|i| i.runtime == "MLX" && i.name == "mlx-community/Qwen2.5-7B-4bit"));
        assert!(hf.iter().any(|i| i.runtime == "llama.cpp (GGUF)" && i.name.ends_with("Q4_K_M.gguf")));
        assert_eq!(parse_ollama_list("NAME ID SIZE MODIFIED\nllama3.2:3b abc 2.0 GB 1 day ago\n"), vec!["llama3.2:3b"]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
