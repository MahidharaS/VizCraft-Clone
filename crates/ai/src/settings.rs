//! AI settings, stored per user in `~/.vizardcraft/ai.json` — never in the project file, so a shared
//! `.vizc` never carries anyone's API key.

use serde::{Deserialize, Serialize};

use crate::provider::{ProviderConfig, ProviderKind};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DecisionBackend {
    /// The on-device brain (always available, learns from you).
    #[default]
    Local,
    /// The OpenJEV decision model behind its helper (`/v1/systemone`).
    OpenJev,
    /// The active chat model, asked for constrained JSON.
    Llm,
}

impl DecisionBackend {
    pub fn label(self) -> &'static str {
        match self {
            DecisionBackend::Local => "Personal brain (on device)",
            DecisionBackend::OpenJev => "OpenJEV decision model",
            DecisionBackend::Llm => "Active chat model (JSON)",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OpenJevConfig {
    /// The helper's address (`python helper/shim.py --port 3000`).
    pub base_url: String,
    /// `SHIM_TOKEN` when the helper is exposed beyond loopback.
    pub token: String,
    pub timeout_secs: u64,
}

impl Default for OpenJevConfig {
    fn default() -> Self {
        OpenJevConfig { base_url: "http://127.0.0.1:3000".into(), token: String::new(), timeout_secs: 20 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Privacy {
    /// Record actions in the personal brain (`~/.vizardcraft/brain/events.jsonl`).
    pub learn: bool,
    /// Send sample rows (not just schema) to cloud models.
    pub send_sample_rows: bool,
    /// Max rows a tool returns to the model.
    pub max_rows_to_model: usize,
    /// Apply project changes the copilot proposes without asking.
    pub auto_apply: bool,
    /// The user has seen the learning disclosure.
    pub disclosure_seen: bool,
}

impl Default for Privacy {
    fn default() -> Self {
        Privacy { learn: true, send_sample_rows: true, max_rows_to_model: 50, auto_apply: false, disclosure_seen: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AiSettings {
    pub providers: Vec<ProviderConfig>,
    /// The provider id the copilot and Formulator use ("" = none: rule-based + brain only).
    pub active: String,
    pub decision: DecisionBackend,
    pub openjev: OpenJevConfig,
    pub privacy: Privacy,
    /// Max agent steps (model calls) per question.
    pub max_steps: usize,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            providers: ProviderKind::ALL.iter().map(|k| ProviderConfig::new(*k)).collect(),
            active: String::new(),
            decision: DecisionBackend::Local,
            openjev: OpenJevConfig::default(),
            privacy: Privacy::default(),
            max_steps: 8,
        }
    }
}

impl AiSettings {
    pub fn path() -> std::path::PathBuf {
        crate::home_dir().join("ai.json")
    }

    /// Loads the settings (defaults when missing or unreadable). Providers added in newer versions
    /// are appended so the list stays complete.
    pub fn load() -> AiSettings {
        let mut s: AiSettings = std::fs::read_to_string(Self::path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        for k in ProviderKind::ALL {
            if !s.providers.iter().any(|p| p.kind == k) {
                s.providers.push(ProviderConfig::new(k));
            }
        }
        // Older settings used `localhost`, which can resolve to IPv6 first on macOS while local
        // servers listen on IPv4 only.
        for p in &mut s.providers {
            if p.kind.is_local() && p.base_url.starts_with("http://localhost:") {
                p.base_url = p.base_url.replacen("http://localhost:", "http://127.0.0.1:", 1);
            }
        }
        if s.openjev.base_url.starts_with("http://localhost:") {
            s.openjev.base_url = s.openjev.base_url.replacen("http://localhost:", "http://127.0.0.1:", 1);
        }
        if s.active.is_empty() {
            // First run: pick a provider whose key is in the environment.
            if let Some(p) = s.providers.iter().find(|p| p.kind.needs_key() && !p.resolved_key().is_empty()) {
                s.active = p.id.clone();
            }
        }
        s
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
        // Keys are in this file: keep it private to the user where the OS supports it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn active_provider(&self) -> Option<&ProviderConfig> {
        if self.active.is_empty() {
            return None;
        }
        self.providers.iter().find(|p| p.id == self.active)
    }

    pub fn provider_mut(&mut self, id: &str) -> Option<&mut ProviderConfig> {
        self.providers.iter_mut().find(|p| p.id == id)
    }

    /// True when the active provider can be called (a key where one is needed).
    pub fn has_llm(&self) -> bool {
        self.active_provider().is_some_and(|p| !p.kind.needs_key() || !p.resolved_key().is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_roundtrip_and_fill_missing_providers() {
        let mut s = AiSettings::default();
        s.providers.retain(|p| p.kind != ProviderKind::Gemini);
        s.active = "anthropic".into();
        let text = serde_json::to_string(&s).unwrap();
        let back: AiSettings = serde_json::from_str(&text).unwrap();
        assert_eq!(back.active, "anthropic");
        assert_eq!(back.active_provider().unwrap().model, "claude-sonnet-5-5");
        // Unknown/partial JSON still loads.
        let partial: AiSettings = serde_json::from_str("{\"active\":\"ollama\"}").unwrap();
        assert_eq!(partial.max_steps, 8);
    }
}
