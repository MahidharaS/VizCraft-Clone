//! VizardCraft AI.
//!
//! - [`provider`]: chat providers behind one trait: OpenAI-compatible (OpenAI, Azure, Ollama,
//!   LM Studio, vLLM, llama.cpp, Groq, OpenRouter, Together, Hugging Face router), Anthropic
//!   Messages and Google Gemini; tool calls where the API offers them.
//! - [`detect`]: finds local model servers (Ollama, LM Studio, llama.cpp, vLLM, the OpenJEV helper).
//! - [`decide`]: typed decisions (choice / yes-no / score with probabilities) from the OpenJEV
//!   decision model (`/v1/systemone`), from any chat model via constrained JSON, or from the
//!   on-device [`brain`].
//! - [`tools`] + [`agent`]: the copilot: an agent loop (native function calling, or a JSON tool
//!   protocol for small local models) whose tools read and edit the open project; every step is
//!   traced (a LangSmith-like trace).
//! - [`brain`]: the personal model. An append-only log of what this user does, and learners
//!   trained online, on device, from it (chart-type recommender, formatting preferences,
//!   transform-sequence model, question memory, suggestion bandit).
//! - [`formulate`]: Data Formulator-style derivations: prompt + encodings → fields, DAX and a chart.
//! - [`finetune`]: export the log as an instruction-tuning dataset with a LoRA script.
//!
//! Everything is Rust: the agent/tool loop that LangChain/AutoGen/CrewAI offer in Python is
//! implemented natively here so the desktop app needs no Python runtime.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod agent;
pub mod brain;
pub mod decide;
pub mod detect;
pub mod finetune;
pub mod formulate;
pub mod http;
pub mod installed;
pub mod provider;
pub mod settings;
pub mod tools;

pub use provider::{ChatProvider, ChatRequest, ChatResponse, Message, ProviderConfig, ProviderKind, Role, ToolCall, ToolSpec, Usage};
pub use settings::AiSettings;

#[derive(Clone, Debug, PartialEq)]
pub enum AiError {
    /// Missing key, unconfigured provider, …
    Config(String),
    /// Couldn't connect / timed out.
    Network(String),
    /// The server answered with an error status.
    Http { status: u16, message: String },
    /// The answer wasn't what we expected.
    BadResponse(String),
    /// Stopped by the user.
    Cancelled,
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiError::Config(m) => write!(f, "{m}"),
            AiError::Network(m) => write!(f, "Couldn't reach the model server: {m}"),
            AiError::Http { status, message } => write!(f, "The model server answered {status}: {message}"),
            AiError::BadResponse(m) => write!(f, "Unexpected answer from the model: {m}"),
            AiError::Cancelled => write!(f, "Stopped."),
        }
    }
}

impl std::error::Error for AiError {}

/// `~/.vizardcraft` (or `$VIZARDCRAFT_HOME` when set — tests and portable installs use it).
pub fn home_dir() -> std::path::PathBuf {
    if let Ok(h) = std::env::var("VIZARDCRAFT_HOME")
        && !h.is_empty()
    {
        return std::path::PathBuf::from(h);
    }
    let base = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into());
    std::path::PathBuf::from(base).join(".vizardcraft")
}

/// Extracts the first balanced JSON object from model text (handles ```json fences and prose).
pub fn extract_json(text: &str) -> Option<serde_json::Value> {
    let bytes: Vec<char> = text.chars().collect();
    let mut start = 0;
    while start < bytes.len() {
        let Some(off) = bytes[start..].iter().position(|c| *c == '{') else { return None };
        let s = start + off;
        let mut depth = 0i32;
        let mut in_str = false;
        let mut esc = false;
        for (i, c) in bytes.iter().enumerate().skip(s) {
            if in_str {
                if esc {
                    esc = false;
                } else if *c == '\\' {
                    esc = true;
                } else if *c == '"' {
                    in_str = false;
                }
                continue;
            }
            match c {
                '"' => in_str = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        let cand: String = bytes[s..=i].iter().collect();
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&cand) {
                            return Some(v);
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
        start = s + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn extracts_json_from_prose() {
        let v = super::extract_json("Sure!\n```json\n{\"tool\": \"x\", \"arguments\": {\"a\": \"}\"}}\n```").unwrap();
        assert_eq!(v["tool"], "x");
        assert_eq!(v["arguments"]["a"], "}");
        assert!(super::extract_json("no json { here").is_none());
    }
}
