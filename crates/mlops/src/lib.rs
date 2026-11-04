//! VizardCraft MLOps.
//!
//! - [`Registry`]: an on-disk model registry (`~/.vizardcraft/models/<name>/v<version>.json`, one
//!   JSON artifact per version, MLflow-style stages None / Staging / Production / Archived).
//! - [`server`]: a dependency-free HTTP scoring server (`GET /health`, `GET /models`,
//!   `GET /models/<name>/schema`, `POST /models/<name>/predict`), run by the `vizardcraft-mlserve`
//!   binary or in-process.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod server;

use std::path::{Path, PathBuf};

pub use vizardcraft_core::studio::registry::{self as records, RegisteredModel, Stage};

/// The default registry directory: `~/.vizardcraft/models` (or `$VIZARDCRAFT_MODELS`).
pub fn default_dir() -> PathBuf {
    if let Ok(d) = std::env::var("VIZARDCRAFT_MODELS")
        && !d.is_empty()
    {
        return PathBuf::from(d);
    }
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".vizardcraft").join("models")
}

/// File-system safe model directory name.
pub fn slug(name: &str) -> String {
    let s: String = name.trim().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '-' }).collect();
    let s = s.trim_matches(|c| c == '-' || c == '.').to_string();
    if s.is_empty() { "model".into() } else { s }
}

#[derive(Clone, Debug)]
pub struct Registry {
    pub dir: PathBuf,
}

impl Registry {
    pub fn new(dir: impl Into<PathBuf>) -> Registry {
        Registry { dir: dir.into() }
    }

    pub fn open_default() -> Registry {
        Registry::new(default_dir())
    }

    fn version_path(&self, name: &str, version: u32) -> PathBuf {
        self.dir.join(slug(name)).join(format!("v{version}.json"))
    }

    /// Every registered version (sorted by name, then version).
    pub fn list(&self) -> Vec<RegisteredModel> {
        let mut out = vec![];
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return out };
        for entry in rd.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let Ok(files) = std::fs::read_dir(&p) else { continue };
            for f in files.flatten() {
                let fp = f.path();
                if fp.extension().is_some_and(|e| e == "json")
                    && let Some(m) = load_file(&fp)
                {
                    out.push(m);
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name).then(a.version.cmp(&b.version)));
        out
    }

    /// Distinct model names.
    pub fn names(&self) -> Vec<String> {
        let mut n: Vec<String> = self.list().into_iter().map(|m| m.name).collect();
        n.dedup();
        n
    }

    pub fn get(&self, name: &str, version: u32) -> Option<RegisteredModel> {
        load_file(&self.version_path(name, version)).or_else(|| self.list().into_iter().find(|m| m.name == name && m.version == version))
    }

    /// The version a server answers with: Production, else Staging, else the latest.
    pub fn serving(&self, name: &str) -> Option<RegisteredModel> {
        let all = self.list();
        records::serving_version(&all, name).cloned()
    }

    /// Saves a model; version 0 gets the next free version. Returns the stored version.
    pub fn save(&self, m: &RegisteredModel) -> Result<RegisteredModel, String> {
        let mut m = m.clone();
        if m.name.trim().is_empty() {
            return Err("The model needs a name.".into());
        }
        if m.version == 0 {
            m.version = records::next_version(&self.list(), &m.name);
        }
        let path = self.version_path(&m.name, m.version);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Couldn't create {}: {e}", parent.display()))?;
        }
        write_atomic(&path, &records::to_json(&m))?;
        Ok(m)
    }

    /// Moves a version to a stage. Promoting to Production archives the previous Production
    /// version of the same model.
    pub fn set_stage(&self, name: &str, version: u32, stage: Stage) -> Result<(), String> {
        let mut m = self.get(name, version).ok_or_else(|| format!("No model {name} v{version}."))?;
        if stage == Stage::Production {
            for mut other in self.list().into_iter().filter(|o| o.name == name && o.version != version && o.stage == Stage::Production) {
                other.stage = Stage::Archived;
                self.save(&other)?;
            }
        }
        m.stage = stage;
        self.save(&m).map(|_| ())
    }

    pub fn delete(&self, name: &str, version: u32) -> Result<(), String> {
        let p = self.version_path(name, version);
        std::fs::remove_file(&p).map_err(|e| format!("Couldn't delete {}: {e}", p.display()))?;
        if let Some(dir) = p.parent() {
            let _ = std::fs::remove_dir(dir); // only when empty
        }
        Ok(())
    }

    /// Writes every given model version (e.g. the project's registry) to disk.
    pub fn sync(&self, models: &[RegisteredModel]) -> Result<usize, String> {
        for m in models {
            self.save(m)?;
        }
        Ok(models.len())
    }
}

fn load_file(p: &Path) -> Option<RegisteredModel> {
    let s = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&s).ok()
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("Couldn't write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Couldn't write {}: {e}", path.display()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use vizardcraft_core::build::{ColType, TableData};
    use vizardcraft_core::scalar::Scalar;
    use vizardcraft_core::studio::learn::{self, LearnerSpec};

    pub fn table() -> TableData {
        let rows = (0..120)
            .map(|i| {
                let x = i as f64 / 10.0;
                vec![Scalar::Num(x), Scalar::Text(if i % 3 == 0 { "a".into() } else { "b".into() }), Scalar::Text(if x > 6.0 { "yes".into() } else { "no".into() })]
            })
            .collect();
        TableData { name: "t".into(), columns: vec!["x".into(), "g".into(), "label".into()], types: vec![ColType::Number, ColType::Text, ColType::Text], rows }
    }

    pub fn model(name: &str) -> RegisteredModel {
        let t = table();
        let m = learn::fit(&LearnerSpec { kind: "tree".into(), ..Default::default() }, &t, &["x".into(), "g".into()], "label").unwrap();
        records::register(name, &m, &t, Default::default(), "test", "")
    }

    #[test]
    fn versions_and_stages() {
        let dir = std::env::temp_dir().join(format!("vc-registry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let r = Registry::new(&dir);
        let v1 = r.save(&model("churn model")).unwrap();
        let v2 = r.save(&model("churn model")).unwrap();
        assert_eq!((v1.version, v2.version), (1, 2));
        assert_eq!(r.serving("churn model").unwrap().version, 2);
        r.set_stage("churn model", 1, Stage::Production).unwrap();
        assert_eq!(r.serving("churn model").unwrap().version, 1);
        r.set_stage("churn model", 2, Stage::Production).unwrap();
        assert_eq!(r.get("churn model", 1).unwrap().stage, Stage::Archived);
        r.delete("churn model", 1).unwrap();
        assert_eq!(r.list().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
