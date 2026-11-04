//! Chat providers behind one trait: OpenAI-compatible chat completions (OpenAI, Azure OpenAI,
//! Ollama, LM Studio, vLLM, llama.cpp server, Groq, OpenRouter, Together, Hugging Face Inference
//! router, any custom endpoint), the Anthropic Messages API and Google Gemini.
//!
//! Requests are blocking; run them on a background thread.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::AiError;
use crate::http::post_json;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// For `Role::Tool`: the call this answers, and the tool's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Message {
    pub fn system(s: impl Into<String>) -> Message {
        Message { role: Role::System, content: s.into(), tool_calls: vec![], tool_call_id: None, name: None }
    }
    pub fn user(s: impl Into<String>) -> Message {
        Message { role: Role::User, content: s.into(), tool_calls: vec![], tool_call_id: None, name: None }
    }
    pub fn assistant(s: impl Into<String>) -> Message {
        Message { role: Role::Assistant, content: s.into(), tool_calls: vec![], tool_call_id: None, name: None }
    }
    pub fn tool(call: &ToolCall, result: impl Into<String>) -> Message {
        Message { role: Role::Tool, content: result.into(), tool_calls: vec![], tool_call_id: Some(call.id.clone()), name: Some(call.name.clone()) }
    }
}

/// A tool the model may call: name, description and a JSON schema of its arguments.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub temperature: Option<f32>,
    pub max_tokens: u32,
    /// Ask for a JSON object answer (where the API supports it).
    pub json_mode: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    pub model: String,
    pub stop_reason: String,
}

/// Anything that answers a chat request.
pub trait ChatProvider: Send + Sync {
    fn label(&self) -> String;
    /// The API takes tool definitions and returns structured tool calls.
    fn native_tools(&self) -> bool;
    fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, AiError>;
}

// ============================================================== configuration

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    Gemini,
    HuggingFace,
    AzureOpenAi,
    Groq,
    OpenRouter,
    Together,
    Ollama,
    LmStudio,
    LlamaCpp,
    Vllm,
    /// Any other OpenAI-compatible endpoint.
    Custom,
}

impl ProviderKind {
    pub const ALL: [ProviderKind; 13] = [
        ProviderKind::OpenAi,
        ProviderKind::Anthropic,
        ProviderKind::Gemini,
        ProviderKind::HuggingFace,
        ProviderKind::AzureOpenAi,
        ProviderKind::Groq,
        ProviderKind::OpenRouter,
        ProviderKind::Together,
        ProviderKind::Ollama,
        ProviderKind::LmStudio,
        ProviderKind::LlamaCpp,
        ProviderKind::Vllm,
        ProviderKind::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::OpenAi => "OpenAI",
            ProviderKind::Anthropic => "Anthropic (Claude)",
            ProviderKind::Gemini => "Google Gemini",
            ProviderKind::HuggingFace => "Hugging Face Inference",
            ProviderKind::AzureOpenAi => "Azure OpenAI",
            ProviderKind::Groq => "Groq",
            ProviderKind::OpenRouter => "OpenRouter",
            ProviderKind::Together => "Together AI",
            ProviderKind::Ollama => "Ollama (local)",
            ProviderKind::LmStudio => "LM Studio (local)",
            ProviderKind::LlamaCpp => "llama.cpp server (local)",
            ProviderKind::Vllm => "vLLM (local)",
            ProviderKind::Custom => "Custom OpenAI-compatible",
        }
    }

    pub fn is_local(self) -> bool {
        matches!(self, ProviderKind::Ollama | ProviderKind::LmStudio | ProviderKind::LlamaCpp | ProviderKind::Vllm)
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            ProviderKind::OpenAi => "https://api.openai.com/v1",
            ProviderKind::Anthropic => "https://api.anthropic.com",
            ProviderKind::Gemini => "https://generativelanguage.googleapis.com",
            ProviderKind::HuggingFace => "https://router.huggingface.co/v1",
            ProviderKind::AzureOpenAi => "https://YOUR-RESOURCE.openai.azure.com/openai/deployments/YOUR-DEPLOYMENT",
            ProviderKind::Groq => "https://api.groq.com/openai/v1",
            ProviderKind::OpenRouter => "https://openrouter.ai/api/v1",
            ProviderKind::Together => "https://api.together.xyz/v1",
            ProviderKind::Ollama => "http://localhost:11434/v1",
            ProviderKind::LmStudio => "http://localhost:1234/v1",
            ProviderKind::LlamaCpp => "http://localhost:8080/v1",
            ProviderKind::Vllm => "http://localhost:8000/v1",
            ProviderKind::Custom => "http://localhost:8000/v1",
        }
    }

    /// A sensible current default model id (editable in settings).
    pub fn default_model(self) -> &'static str {
        match self {
            ProviderKind::OpenAi => "gpt-5",
            ProviderKind::Anthropic => "claude-sonnet-5-5",
            ProviderKind::Gemini => "gemini-2.5-pro",
            ProviderKind::HuggingFace => "Qwen/Qwen2.5-72B-Instruct",
            ProviderKind::AzureOpenAi => "gpt-5",
            ProviderKind::Groq => "llama-3.3-70b-versatile",
            ProviderKind::OpenRouter => "anthropic/claude-sonnet-5-5",
            ProviderKind::Together => "meta-llama/Llama-3.3-70B-Instruct-Turbo",
            ProviderKind::Ollama => "qwen2.5:7b-instruct",
            ProviderKind::LmStudio => "qwen2.5-7b-instruct",
            ProviderKind::LlamaCpp => "default",
            ProviderKind::Vllm => "Qwen/Qwen2.5-7B-Instruct",
            ProviderKind::Custom => "default",
        }
    }

    /// The environment variable read when no key is stored.
    pub fn env_key(self) -> Option<&'static str> {
        match self {
            ProviderKind::OpenAi => Some("OPENAI_API_KEY"),
            ProviderKind::Anthropic => Some("ANTHROPIC_API_KEY"),
            ProviderKind::Gemini => Some("GEMINI_API_KEY"),
            ProviderKind::HuggingFace => Some("HF_TOKEN"),
            ProviderKind::AzureOpenAi => Some("AZURE_OPENAI_API_KEY"),
            ProviderKind::Groq => Some("GROQ_API_KEY"),
            ProviderKind::OpenRouter => Some("OPENROUTER_API_KEY"),
            ProviderKind::Together => Some("TOGETHER_API_KEY"),
            _ => None,
        }
    }

    pub fn needs_key(self) -> bool {
        !self.is_local() && self != ProviderKind::Custom
    }

    pub fn wire(self) -> Wire {
        match self {
            ProviderKind::Anthropic => Wire::Anthropic,
            ProviderKind::Gemini => Wire::Gemini,
            ProviderKind::AzureOpenAi => Wire::Azure,
            _ => Wire::OpenAi,
        }
    }
}

/// Which wire protocol a provider speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wire {
    OpenAi,
    Azure,
    Anthropic,
    Gemini,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderConfig {
    pub id: String,
    pub kind: ProviderKind,
    pub label: String,
    pub base_url: String,
    /// Stored key (empty → the provider's environment variable).
    pub api_key: String,
    pub model: String,
    /// Send tool definitions natively (off → the JSON tool protocol, for small local models).
    pub native_tools: bool,
    /// Azure OpenAI `api-version`.
    pub api_version: String,
    pub temperature: f32,
    pub max_tokens: u32,
    pub timeout_secs: u64,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        ProviderConfig::new(ProviderKind::OpenAi)
    }
}

impl ProviderConfig {
    pub fn new(kind: ProviderKind) -> ProviderConfig {
        ProviderConfig {
            id: format!("{kind:?}").to_lowercase(),
            kind,
            label: kind.label().to_string(),
            base_url: kind.default_base_url().to_string(),
            api_key: String::new(),
            model: kind.default_model().to_string(),
            native_tools: !matches!(kind, ProviderKind::LlamaCpp | ProviderKind::Custom),
            api_version: "2024-10-21".into(),
            temperature: 0.2,
            max_tokens: 2048,
            timeout_secs: if kind.is_local() { 300 } else { 120 },
        }
    }

    /// The key to use: the stored one, else the environment variable.
    pub fn resolved_key(&self) -> String {
        if !self.api_key.trim().is_empty() {
            return self.api_key.trim().to_string();
        }
        self.kind.env_key().and_then(|k| std::env::var(k).ok()).unwrap_or_default()
    }

    pub fn key_source(&self) -> &'static str {
        if !self.api_key.trim().is_empty() {
            "stored in ~/.vizcraft/ai.json"
        } else if self.kind.env_key().and_then(|k| std::env::var(k).ok()).is_some_and(|v| !v.is_empty()) {
            "from environment"
        } else if self.kind.needs_key() {
            "missing"
        } else {
            "not needed"
        }
    }

    pub fn build(&self) -> Box<dyn ChatProvider> {
        Box::new(HttpProvider { cfg: self.clone() })
    }
}

/// The HTTP implementation of every provider kind.
pub struct HttpProvider {
    pub cfg: ProviderConfig,
}

impl ChatProvider for HttpProvider {
    fn label(&self) -> String {
        format!("{} · {}", self.cfg.label, self.cfg.model)
    }
    fn native_tools(&self) -> bool {
        self.cfg.native_tools
    }
    fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, AiError> {
        let key = self.cfg.resolved_key();
        if self.cfg.kind.needs_key() && key.is_empty() {
            return Err(AiError::Config(format!(
                "{} needs an API key: paste it in AI settings or set {}.",
                self.cfg.label,
                self.cfg.kind.env_key().unwrap_or("the key variable")
            )));
        }
        let timeout = Duration::from_secs(self.cfg.timeout_secs.max(5));
        let base = self.cfg.base_url.trim_end_matches('/');
        match self.cfg.kind.wire() {
            Wire::OpenAi | Wire::Azure => {
                let (url, headers) = if self.cfg.kind.wire() == Wire::Azure {
                    (format!("{base}/chat/completions?api-version={}", self.cfg.api_version), vec![("api-key".to_string(), key)])
                } else {
                    let mut h = vec![];
                    if !key.is_empty() {
                        h.push(("Authorization".to_string(), format!("Bearer {key}")));
                    }
                    if self.cfg.kind == ProviderKind::OpenRouter {
                        h.push(("X-Title".to_string(), "VizCraft".to_string()));
                    }
                    (format!("{base}/chat/completions"), h)
                };
                let body = openai_body(&self.cfg, req);
                let v = post_json(&url, &headers, &body, timeout)?;
                parse_openai(&v)
            }
            Wire::Anthropic => {
                let url = format!("{base}/v1/messages");
                let headers = vec![("x-api-key".to_string(), key), ("anthropic-version".to_string(), "2023-06-01".to_string())];
                let v = post_json(&url, &headers, &anthropic_body(&self.cfg, req), timeout)?;
                parse_anthropic(&v)
            }
            Wire::Gemini => {
                let url = format!("{base}/v1beta/models/{}:generateContent", self.cfg.model);
                let headers = vec![("x-goog-api-key".to_string(), key)];
                let v = post_json(&url, &headers, &gemini_body(&self.cfg, req), timeout)?;
                parse_gemini(&v)
            }
        }
    }
}

// ============================================================== OpenAI-compatible

pub fn openai_body(cfg: &ProviderConfig, req: &ChatRequest) -> Value {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .map(|m| match m.role {
            Role::System => json!({"role": "system", "content": m.content}),
            Role::User => json!({"role": "user", "content": m.content}),
            Role::Assistant => {
                let mut o = json!({"role": "assistant", "content": m.content});
                if !m.tool_calls.is_empty() {
                    o["tool_calls"] = Value::Array(
                        m.tool_calls.iter().map(|c| json!({"id": c.id, "type": "function", "function": {"name": c.name, "arguments": c.arguments.to_string()}})).collect(),
                    );
                }
                o
            }
            Role::Tool => json!({"role": "tool", "tool_call_id": m.tool_call_id.clone().unwrap_or_default(), "content": m.content}),
        })
        .collect();
    let mut body = json!({"model": cfg.model, "messages": messages});
    // OpenAI's reasoning models take `max_completion_tokens` and only the default temperature.
    if matches!(cfg.kind, ProviderKind::OpenAi | ProviderKind::AzureOpenAi) {
        body["max_completion_tokens"] = json!(req.max_tokens.max(1));
    } else {
        body["max_tokens"] = json!(req.max_tokens.max(1));
        if let Some(t) = req.temperature {
            body["temperature"] = json!(t);
        }
    }
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(req.tools.iter().map(|t| json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.parameters}})).collect());
    }
    if req.json_mode && req.tools.is_empty() {
        body["response_format"] = json!({"type": "json_object"});
    }
    body
}

pub fn parse_openai(v: &Value) -> Result<ChatResponse, AiError> {
    let choice = v.pointer("/choices/0").ok_or_else(|| AiError::BadResponse("no choices in the answer".into()))?;
    let msg = choice.get("message").cloned().unwrap_or(Value::Null);
    let content = match msg.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join(""),
        _ => String::new(),
    };
    let mut tool_calls = Vec::new();
    if let Some(Value::Array(calls)) = msg.get("tool_calls") {
        for (i, c) in calls.iter().enumerate() {
            let name = c.pointer("/function/name").and_then(Value::as_str).unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            let args = match c.pointer("/function/arguments") {
                Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_else(|_| json!({"_raw": s})),
                Some(o @ Value::Object(_)) => o.clone(),
                _ => json!({}),
            };
            let id = c.get("id").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("call_{i}"));
            tool_calls.push(ToolCall { id, name, arguments: args });
        }
    }
    Ok(ChatResponse {
        content,
        tool_calls,
        usage: Usage {
            input_tokens: v.pointer("/usage/prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
            output_tokens: v.pointer("/usage/completion_tokens").and_then(Value::as_u64).unwrap_or(0),
        },
        model: v.get("model").and_then(Value::as_str).unwrap_or("").to_string(),
        stop_reason: choice.get("finish_reason").and_then(Value::as_str).unwrap_or("").to_string(),
    })
}

// ============================================================== Anthropic

pub fn anthropic_body(cfg: &ProviderConfig, req: &ChatRequest) -> Value {
    let system: Vec<&str> = req.messages.iter().filter(|m| m.role == Role::System).map(|m| m.content.as_str()).collect();
    let mut messages: Vec<Value> = Vec::new();
    for m in req.messages.iter().filter(|m| m.role != Role::System) {
        let (role, block) = match m.role {
            Role::User => ("user", vec![json!({"type": "text", "text": m.content})]),
            Role::Assistant => {
                let mut blocks = vec![];
                if !m.content.is_empty() {
                    blocks.push(json!({"type": "text", "text": m.content}));
                }
                for c in &m.tool_calls {
                    blocks.push(json!({"type": "tool_use", "id": c.id, "name": c.name, "input": if c.arguments.is_object() { c.arguments.clone() } else { json!({}) }}));
                }
                if blocks.is_empty() {
                    blocks.push(json!({"type": "text", "text": "…"}));
                }
                ("assistant", blocks)
            }
            Role::Tool => ("user", vec![json!({"type": "tool_result", "tool_use_id": m.tool_call_id.clone().unwrap_or_default(), "content": m.content})]),
            Role::System => continue,
        };
        // Consecutive same-role turns (several tool results) merge into one message.
        if let Some(last) = messages.last_mut()
            && last["role"] == role
            && let Some(Value::Array(c)) = last.get_mut("content")
        {
            c.extend(block);
            continue;
        }
        messages.push(json!({"role": role, "content": block}));
    }
    let mut body = json!({"model": cfg.model, "max_tokens": req.max_tokens.max(1), "messages": messages});
    if !system.is_empty() {
        body["system"] = json!(system.join("\n\n"));
    }
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(req.tools.iter().map(|t| json!({"name": t.name, "description": t.description, "input_schema": t.parameters})).collect());
    }
    body
}

pub fn parse_anthropic(v: &Value) -> Result<ChatResponse, AiError> {
    let Some(Value::Array(blocks)) = v.get("content") else { return Err(AiError::BadResponse("no content in the answer".into())) };
    let mut content = String::new();
    let mut tool_calls = Vec::new();
    for b in blocks {
        match b.get("type").and_then(Value::as_str) {
            Some("text") => content.push_str(b.get("text").and_then(Value::as_str).unwrap_or("")),
            Some("tool_use") => tool_calls.push(ToolCall {
                id: b.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
                name: b.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                arguments: b.get("input").cloned().unwrap_or(json!({})),
            }),
            _ => {}
        }
    }
    Ok(ChatResponse {
        content,
        tool_calls,
        usage: Usage {
            input_tokens: v.pointer("/usage/input_tokens").and_then(Value::as_u64).unwrap_or(0),
            output_tokens: v.pointer("/usage/output_tokens").and_then(Value::as_u64).unwrap_or(0),
        },
        model: v.get("model").and_then(Value::as_str).unwrap_or("").to_string(),
        stop_reason: v.get("stop_reason").and_then(Value::as_str).unwrap_or("").to_string(),
    })
}

// ============================================================== Gemini

/// Gemini's schema dialect is a subset of JSON schema: drop the keys it rejects.
fn gemini_schema(v: &Value) -> Value {
    match v {
        Value::Object(o) => Value::Object(o.iter().filter(|(k, _)| !matches!(k.as_str(), "additionalProperties" | "$schema" | "default")).map(|(k, v)| (k.clone(), gemini_schema(v))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(gemini_schema).collect()),
        x => x.clone(),
    }
}

pub fn gemini_body(cfg: &ProviderConfig, req: &ChatRequest) -> Value {
    let system: Vec<&str> = req.messages.iter().filter(|m| m.role == Role::System).map(|m| m.content.as_str()).collect();
    let mut contents: Vec<Value> = Vec::new();
    for m in req.messages.iter().filter(|m| m.role != Role::System) {
        let (role, parts) = match m.role {
            Role::User => ("user", vec![json!({"text": m.content})]),
            Role::Assistant => {
                let mut p = vec![];
                if !m.content.is_empty() {
                    p.push(json!({"text": m.content}));
                }
                for c in &m.tool_calls {
                    p.push(json!({"functionCall": {"name": c.name, "args": c.arguments}}));
                }
                ("model", p)
            }
            Role::Tool => {
                let resp = serde_json::from_str::<Value>(&m.content).ok().filter(Value::is_object).unwrap_or_else(|| json!({"result": m.content}));
                ("user", vec![json!({"functionResponse": {"name": m.name.clone().unwrap_or_default(), "response": resp}})])
            }
            Role::System => continue,
        };
        if parts.is_empty() {
            continue;
        }
        if let Some(last) = contents.last_mut()
            && last["role"] == role
            && let Some(Value::Array(p)) = last.get_mut("parts")
        {
            p.extend(parts);
            continue;
        }
        contents.push(json!({"role": role, "parts": parts}));
    }
    let mut gen_cfg = json!({"maxOutputTokens": req.max_tokens.max(1)});
    if let Some(t) = req.temperature {
        gen_cfg["temperature"] = json!(t);
    }
    if req.json_mode && req.tools.is_empty() {
        gen_cfg["responseMimeType"] = json!("application/json");
    }
    let mut body = json!({"contents": contents, "generationConfig": gen_cfg});
    if !system.is_empty() {
        body["systemInstruction"] = json!({"parts": [{"text": system.join("\n\n")}]});
    }
    if !req.tools.is_empty() {
        body["tools"] = json!([{"functionDeclarations": req.tools.iter().map(|t| json!({"name": t.name, "description": t.description, "parameters": gemini_schema(&t.parameters)})).collect::<Vec<_>>()}]);
    }
    let _ = cfg;
    body
}

pub fn parse_gemini(v: &Value) -> Result<ChatResponse, AiError> {
    let cand = v.pointer("/candidates/0").ok_or_else(|| {
        let why = v.pointer("/promptFeedback/blockReason").and_then(Value::as_str).unwrap_or("no candidates");
        AiError::BadResponse(format!("Gemini returned no answer ({why})"))
    })?;
    let mut content = String::new();
    let mut tool_calls = Vec::new();
    if let Some(Value::Array(parts)) = cand.pointer("/content/parts") {
        for (i, p) in parts.iter().enumerate() {
            if let Some(t) = p.get("text").and_then(Value::as_str) {
                content.push_str(t);
            }
            if let Some(fc) = p.get("functionCall") {
                tool_calls.push(ToolCall {
                    id: format!("gemini_{i}"),
                    name: fc.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                    arguments: fc.get("args").cloned().unwrap_or(json!({})),
                });
            }
        }
    }
    Ok(ChatResponse {
        content,
        tool_calls,
        usage: Usage {
            input_tokens: v.pointer("/usageMetadata/promptTokenCount").and_then(Value::as_u64).unwrap_or(0),
            output_tokens: v.pointer("/usageMetadata/candidatesTokenCount").and_then(Value::as_u64).unwrap_or(0),
        },
        model: v.get("modelVersion").and_then(Value::as_str).unwrap_or("").to_string(),
        stop_reason: cand.get("finishReason").and_then(Value::as_str).unwrap_or("").to_string(),
    })
}

/// A provider that replays scripted answers (tests, demos, offline UI checks).
pub struct ScriptedProvider {
    pub answers: std::sync::Mutex<std::collections::VecDeque<ChatResponse>>,
    pub seen: std::sync::Mutex<Vec<ChatRequest>>,
    pub native: bool,
}

impl ScriptedProvider {
    pub fn new(answers: Vec<ChatResponse>, native: bool) -> ScriptedProvider {
        ScriptedProvider { answers: std::sync::Mutex::new(answers.into()), seen: std::sync::Mutex::new(vec![]), native }
    }
}

impl ChatProvider for ScriptedProvider {
    fn label(&self) -> String {
        "Scripted".into()
    }
    fn native_tools(&self) -> bool {
        self.native
    }
    fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, AiError> {
        if let Ok(mut s) = self.seen.lock() {
            s.push(req.clone());
        }
        self.answers.lock().ok().and_then(|mut a| a.pop_front()).ok_or_else(|| AiError::BadResponse("script exhausted".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::mock;

    fn tool() -> ToolSpec {
        ToolSpec { name: "run_dax".into(), description: "Run a DAX query".into(), parameters: json!({"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}) }
    }

    fn convo() -> ChatRequest {
        let call = ToolCall { id: "c1".into(), name: "run_dax".into(), arguments: json!({"query": "EVALUATE Sales"}) };
        let mut a = Message::assistant("");
        a.tool_calls.push(call.clone());
        ChatRequest {
            messages: vec![Message::system("You are VizCraft."), Message::user("Total sales?"), a, Message::tool(&call, "{\"rows\":[[1]]}")],
            tools: vec![tool()],
            temperature: Some(0.1),
            max_tokens: 100,
            json_mode: false,
        }
    }

    #[test]
    fn openai_roundtrip_with_tool_calls() {
        let answer = json!({
            "model": "gpt-test",
            "choices": [{"finish_reason": "tool_calls", "message": {"role": "assistant", "content": null,
                "tool_calls": [{"id": "call_9", "type": "function", "function": {"name": "run_dax", "arguments": "{\"query\":\"EVALUATE ROW(\\\"x\\\", 1)\"}"}}]}}],
            "usage": {"prompt_tokens": 12, "completion_tokens": 7}
        });
        let m = mock::serve(vec![(200, answer.to_string())]);
        let mut cfg = ProviderConfig::new(ProviderKind::Custom);
        cfg.base_url = format!("{}/v1", m.url);
        cfg.api_key = "sk-test".into();
        cfg.model = "gpt-test".into();
        let r = cfg.build().chat(&convo()).expect("chat");
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].name, "run_dax");
        assert_eq!(r.tool_calls[0].arguments["query"], "EVALUATE ROW(\"x\", 1)");
        assert_eq!(r.usage, Usage { input_tokens: 12, output_tokens: 7 });
        let rec = m.received.lock().expect("lock");
        assert_eq!(rec[0].path, "/v1/chat/completions");
        assert!(rec[0].headers.iter().any(|(k, v)| k == "authorization" && v == "Bearer sk-test"));
        let body: Value = serde_json::from_str(&rec[0].body).expect("json");
        assert_eq!(body["model"], "gpt-test");
        assert_eq!(body["tools"][0]["function"]["name"], "run_dax");
        assert_eq!(body["messages"][2]["tool_calls"][0]["function"]["name"], "run_dax");
        assert_eq!(body["messages"][3]["role"], "tool");
        assert_eq!(body["messages"][3]["tool_call_id"], "c1");
        assert_eq!(body["temperature"], json!(0.1f32));
    }

    #[test]
    fn anthropic_roundtrip() {
        let answer = json!({"model": "claude-sonnet-5-5", "stop_reason": "tool_use",
            "content": [{"type": "text", "text": "Let me check."}, {"type": "tool_use", "id": "tu_1", "name": "run_dax", "input": {"query": "EVALUATE Sales"}}],
            "usage": {"input_tokens": 30, "output_tokens": 9}});
        let m = mock::serve(vec![(200, answer.to_string())]);
        let mut cfg = ProviderConfig::new(ProviderKind::Anthropic);
        cfg.base_url = m.url.clone();
        cfg.api_key = "ak".into();
        let r = cfg.build().chat(&convo()).expect("chat");
        assert_eq!(r.content, "Let me check.");
        assert_eq!(r.tool_calls[0].id, "tu_1");
        let rec = m.received.lock().expect("lock");
        assert_eq!(rec[0].path, "/v1/messages");
        assert!(rec[0].headers.iter().any(|(k, v)| k == "x-api-key" && v == "ak"));
        assert!(rec[0].headers.iter().any(|(k, _)| k == "anthropic-version"));
        let body: Value = serde_json::from_str(&rec[0].body).expect("json");
        assert_eq!(body["system"], "You are VizCraft.");
        assert_eq!(body["model"], "claude-sonnet-5-5");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "c1");
    }

    #[test]
    fn gemini_roundtrip() {
        let answer = json!({"candidates": [{"finishReason": "STOP", "content": {"role": "model", "parts": [{"functionCall": {"name": "run_dax", "args": {"query": "EVALUATE Sales"}}}]}}],
            "usageMetadata": {"promptTokenCount": 5, "candidatesTokenCount": 3}});
        let m = mock::serve(vec![(200, answer.to_string())]);
        let mut cfg = ProviderConfig::new(ProviderKind::Gemini);
        cfg.base_url = m.url.clone();
        cfg.api_key = "gk".into();
        cfg.model = "gemini-test".into();
        let r = cfg.build().chat(&convo()).expect("chat");
        assert_eq!(r.tool_calls[0].name, "run_dax");
        assert_eq!(r.usage.output_tokens, 3);
        let rec = m.received.lock().expect("lock");
        assert_eq!(rec[0].path, "/v1beta/models/gemini-test:generateContent");
        let body: Value = serde_json::from_str(&rec[0].body).expect("json");
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "You are VizCraft.");
        assert_eq!(body["contents"][1]["role"], "model");
        assert_eq!(body["contents"][2]["parts"][0]["functionResponse"]["name"], "run_dax");
        assert!(body["tools"][0]["functionDeclarations"][0]["parameters"].get("required").is_some());
    }

    #[test]
    fn http_errors_and_garbage_never_panic() {
        let m = mock::serve(vec![(401, "{\"error\":{\"message\":\"bad key\"}}".into()), (200, "not json".into()), (200, "{\"choices\":[]}".into())]);
        let mut cfg = ProviderConfig::new(ProviderKind::Custom);
        cfg.base_url = m.url.clone();
        let p = cfg.build();
        match p.chat(&convo()) {
            Err(AiError::Http { status, message }) => {
                assert_eq!(status, 401);
                assert_eq!(message, "bad key");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(p.chat(&convo()), Err(AiError::BadResponse(_))));
        assert!(matches!(p.chat(&convo()), Err(AiError::BadResponse(_))));
    }

    #[test]
    fn missing_key_is_a_config_error() {
        let mut cfg = ProviderConfig::new(ProviderKind::Groq);
        cfg.api_key.clear();
        if std::env::var("GROQ_API_KEY").is_err() {
            assert!(matches!(cfg.build().chat(&convo()), Err(AiError::Config(_))));
        }
    }
}
