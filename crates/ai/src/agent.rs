//! The copilot's agent loop: model → tool calls → results → model … → answer.
//!
//! Providers with native function calling get the tool schemas; small local models get the
//! JSON tool protocol instead (one JSON object per turn: `{"tool": …, "arguments": …}` or
//! `{"final": …}`). Every model call and tool call is recorded in a [`Trace`] — the
//! LangSmith-style run log the app's trace viewer shows (inputs, outputs, latency, tokens).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::provider::{ChatProvider, ChatRequest, Message, Role, ToolCall, ToolSpec, Usage};
use crate::tools::ToolOutcome;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepKind {
    Model,
    Tool,
    Answer,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceStep {
    pub kind: StepKind,
    pub name: String,
    pub input: String,
    pub output: String,
    pub latency_ms: f64,
    pub tokens: Usage,
    pub ok: bool,
    /// Unix seconds.
    pub at: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Trace {
    pub id: String,
    pub question: String,
    pub provider: String,
    pub steps: Vec<TraceStep>,
    pub total_ms: f64,
    pub tokens: Usage,
    pub answer: String,
    pub ok: bool,
}

/// Runs tools for the agent (the app forwards calls to its UI thread; tests execute directly).
pub trait ToolExecutor {
    fn execute(&mut self, call: &ToolCall) -> ToolOutcome;
}

impl<F: FnMut(&ToolCall) -> ToolOutcome> ToolExecutor for F {
    fn execute(&mut self, call: &ToolCall) -> ToolOutcome {
        self(call)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentEvent {
    Step(TraceStep),
    Done(Trace),
}

pub const SYSTEM_PROMPT: &str = "You are VizCraft Copilot, the assistant built into VizCraft, a desktop analytics studio \
(Power BI + Tableau + Excel + Fabric + an Orange-style ML studio). You can operate the whole app with tools, \
like a skilled analyst at the keyboard: the semantic model (measures, columns, relationships, TMDL), Power Query (M), \
report pages and visuals, text, shapes and themes, filters, data sheets, Studio ML workflows, and navigation. \
Rules: call describe_model (or get_project with path '') before using names you haven't seen; use exact names \
('Table'[Column], [Measure]); validate numbers with run_dax before stating them; prefer the specific tools \
(add_measure, create_visual, update_query, add_relationship, set_cells…) and use edit_project for anything else \
after reading the JSON with get_project; use navigate to show the user the result; keep answers short and concrete; \
when you change something, say what you did. Never invent data.";

fn unix_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// The JSON tool protocol instructions appended to the system prompt for models without
/// native tool calling.
pub fn json_protocol(tools: &[ToolSpec]) -> String {
    let mut s = String::from(
        "\n\nTOOLS. To use a tool, reply with ONLY one JSON object: {\"tool\": \"<name>\", \"arguments\": {...}}. \
         You will get the result and can call another tool. When you are done, reply with ONLY {\"final\": \"<your answer to the user>\"}.\nAvailable tools:\n",
    );
    for t in tools {
        let args: Vec<String> = t.parameters.get("properties").and_then(Value::as_object).map(|o| o.keys().cloned().collect()).unwrap_or_default();
        s.push_str(&format!("- {}({}): {}\n", t.name, args.join(", "), t.description));
    }
    s
}

/// Parses a JSON-protocol turn: a tool call, a final answer, or plain text (treated as final).
pub fn parse_protocol(content: &str, step: usize) -> Result<ToolCall, String> {
    match crate::extract_json(content) {
        Some(v) if v.get("tool").and_then(Value::as_str).is_some() => Ok(ToolCall {
            id: format!("json_{step}"),
            name: v.get("tool").and_then(Value::as_str).unwrap_or("").to_string(),
            arguments: v.get("arguments").or_else(|| v.get("args")).cloned().unwrap_or(json!({})),
        }),
        Some(v) if v.get("final").is_some() || v.get("answer").is_some() => {
            Err(v.get("final").or_else(|| v.get("answer")).map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).unwrap_or_default())
        }
        _ => Err(content.trim().to_string()),
    }
}

pub struct AgentRun<'a> {
    pub provider: &'a dyn ChatProvider,
    pub tools: Vec<ToolSpec>,
    /// Extra system context: the model summary, the user's habits from the brain.
    pub context: String,
    pub max_steps: usize,
    pub cancel: &'a AtomicBool,
}

impl AgentRun<'_> {
    /// Answers `question` given earlier `history` (user/assistant turns). Returns the trace;
    /// `on_event` sees every step as it happens.
    pub fn run(&self, history: &[Message], question: &str, exec: &mut dyn ToolExecutor, on_event: &mut dyn FnMut(AgentEvent)) -> Trace {
        let t0 = Instant::now();
        let native = self.provider.native_tools();
        let mut system = String::from(SYSTEM_PROMPT);
        if !self.context.is_empty() {
            system.push_str("\n\n");
            system.push_str(&self.context);
        }
        if !native {
            system.push_str(&json_protocol(&self.tools));
        }
        let mut messages = vec![Message::system(system)];
        messages.extend(history.iter().filter(|m| matches!(m.role, Role::User | Role::Assistant) && m.tool_calls.is_empty()).cloned());
        messages.push(Message::user(question));
        let mut trace = Trace { id: format!("run-{}", (unix_now() * 1000.0) as u64), question: question.to_string(), provider: self.provider.label(), ..Trace::default() };
        let push = |trace: &mut Trace, s: TraceStep, on_event: &mut dyn FnMut(AgentEvent)| {
            trace.tokens.input_tokens += s.tokens.input_tokens;
            trace.tokens.output_tokens += s.tokens.output_tokens;
            on_event(AgentEvent::Step(s.clone()));
            trace.steps.push(s);
        };
        let mut answer: Option<String> = None;
        for step in 0..self.max_steps.max(1) {
            if self.cancel.load(Ordering::Relaxed) {
                push(&mut trace, TraceStep { kind: StepKind::Error, name: "cancelled".into(), input: String::new(), output: "Stopped by the user.".into(), latency_ms: 0.0, tokens: Usage::default(), ok: false, at: unix_now() }, on_event);
                answer = Some("Stopped.".into());
                break;
            }
            let req = ChatRequest { messages: messages.clone(), tools: if native { self.tools.clone() } else { vec![] }, temperature: Some(0.2), max_tokens: 2048, json_mode: false };
            let t = Instant::now();
            let resp = self.provider.chat(&req);
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    push(&mut trace, TraceStep { kind: StepKind::Error, name: "model".into(), input: question.into(), output: e.to_string(), latency_ms: ms, tokens: Usage::default(), ok: false, at: unix_now() }, on_event);
                    trace.ok = false;
                    trace.answer = e.to_string();
                    trace.total_ms = t0.elapsed().as_secs_f64() * 1000.0;
                    on_event(AgentEvent::Done(trace.clone()));
                    return trace;
                }
            };
            let mut calls = resp.tool_calls.clone();
            let mut final_text = None;
            if !native {
                match parse_protocol(&resp.content, step) {
                    Ok(c) => calls = vec![c],
                    Err(text) => final_text = Some(text),
                }
            } else if calls.is_empty() {
                final_text = Some(resp.content.trim().to_string());
            }
            let out_summary = if calls.is_empty() { resp.content.clone() } else { calls.iter().map(|c| format!("→ {}({})", c.name, c.arguments)).collect::<Vec<_>>().join("\n") };
            push(
                &mut trace,
                TraceStep { kind: StepKind::Model, name: format!("model call {}", step + 1), input: last_input(&messages), output: out_summary, latency_ms: ms, tokens: resp.usage, ok: true, at: unix_now() },
                on_event,
            );
            if let Some(text) = final_text {
                answer = Some(text);
                break;
            }
            // Record the assistant turn, then run each call.
            let mut a = Message::assistant(if native { resp.content.clone() } else { resp.content.trim().to_string() });
            if native {
                a.tool_calls = calls.clone();
            }
            messages.push(a);
            for c in &calls {
                let t = Instant::now();
                let out = exec.execute(c);
                let ms = t.elapsed().as_secs_f64() * 1000.0;
                push(
                    &mut trace,
                    TraceStep { kind: StepKind::Tool, name: c.name.clone(), input: c.arguments.to_string(), output: out.text.clone(), latency_ms: ms, tokens: Usage::default(), ok: out.ok, at: unix_now() },
                    on_event,
                );
                let clipped: String = out.text.chars().take(12_000).collect();
                if native {
                    messages.push(Message::tool(c, clipped));
                } else {
                    messages.push(Message::user(format!("Result of {}: {}", c.name, clipped)));
                }
            }
        }
        let answer = answer.unwrap_or_else(|| "I ran out of steps before finishing. Ask me to continue, or narrow the question.".into());
        push(&mut trace, TraceStep { kind: StepKind::Answer, name: "answer".into(), input: String::new(), output: answer.clone(), latency_ms: 0.0, tokens: Usage::default(), ok: true, at: unix_now() }, on_event);
        trace.answer = answer;
        trace.ok = true;
        trace.total_ms = t0.elapsed().as_secs_f64() * 1000.0;
        on_event(AgentEvent::Done(trace.clone()));
        trace
    }
}

fn last_input(messages: &[Message]) -> String {
    messages.last().map(|m| m.content.chars().take(2000).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ChatResponse, ScriptedProvider};
    use crate::tools::{self, Workspace};
    use vizcraft_core::build;

    fn sample() -> (vizcraft_core::doc::Project, build::Built, u64) {
        let mut p = vizcraft_core::sample::sales();
        build::sync_model_tables(&mut p);
        let b = build::build(&p, &vizcraft_connectors::DesktopHost::default(), &tools::params(&p));
        let page = p.report.pages.first().map(|x| x.id).unwrap_or(0);
        (p, b, page)
    }

    fn call(name: &str, args: Value) -> ToolCall {
        ToolCall { id: format!("c_{name}"), name: name.into(), arguments: args }
    }

    #[test]
    fn native_tool_loop_against_the_sample_project() {
        let (mut p, mut b, mut page) = sample();
        let n0 = p.page(page).map(|x| x.elements.len()).unwrap_or(0);
        let script = vec![
            ChatResponse { tool_calls: vec![call("describe_model", json!({}))], usage: Usage { input_tokens: 100, output_tokens: 5 }, ..Default::default() },
            ChatResponse {
                tool_calls: vec![call("run_dax", json!({"query": "EVALUATE SUMMARIZECOLUMNS(Store[Region], \"Total\", [Total Sales])"})), call("create_visual", json!({"kind": "clusteredColumn", "axis": ["Store[Region]"], "values": ["[Total Sales]"]}))],
                ..Default::default()
            },
            ChatResponse { content: "Sales by region is on the page.".into(), usage: Usage { input_tokens: 50, output_tokens: 8 }, ..Default::default() },
        ];
        let prov = ScriptedProvider::new(script, true);
        let cancel = AtomicBool::new(false);
        let run = AgentRun { provider: &prov, tools: tools::specs(), context: "User habits: likes column charts.".into(), max_steps: 6, cancel: &cancel };
        let mut events = 0;
        let trace = {
            let mut exec = |c: &ToolCall| {
                let mut ws = Workspace { project: &mut p, built: &mut b, page: &mut page, selected: None, max_rows: 20, send_rows: true };
                tools::execute(&mut ws, c)
            };
            run.run(&[], "Show sales by region", &mut exec, &mut |_| events += 1)
        };
        assert!(trace.ok);
        assert_eq!(trace.answer, "Sales by region is on the page.");
        assert_eq!(trace.tokens.input_tokens, 150);
        let tool_steps: Vec<&TraceStep> = trace.steps.iter().filter(|s| s.kind == StepKind::Tool).collect();
        assert_eq!(tool_steps.len(), 3);
        assert!(tool_steps.iter().all(|s| s.ok), "{tool_steps:?}");
        assert_eq!(p.page(page).map(|x| x.elements.len()).unwrap_or(0), n0 + 1);
        assert!(events >= trace.steps.len());
        // The second model call saw the tool results with matching ids.
        let seen = prov.seen.lock().unwrap();
        let second = &seen[1];
        assert!(second.messages.iter().any(|m| m.role == Role::Tool && m.tool_call_id.as_deref() == Some("c_describe_model")));
        assert!(second.messages[0].content.contains("likes column charts"));
        assert!(!second.tools.is_empty());
    }

    #[test]
    fn json_protocol_loop_for_local_models() {
        let (mut p, mut b, mut page) = sample();
        let script = vec![
            ChatResponse { content: "I'll add it.\n{\"tool\": \"add_measure\", \"arguments\": {\"name\": \"Avg Sale\", \"dax\": \"AVERAGE(Sales[Quantity])\"}}".into(), ..Default::default() },
            ChatResponse { content: "{\"final\": \"Added [Avg Sale].\"}".into(), ..Default::default() },
        ];
        let prov = ScriptedProvider::new(script, false);
        let cancel = AtomicBool::new(false);
        let run = AgentRun { provider: &prov, tools: tools::specs(), context: String::new(), max_steps: 4, cancel: &cancel };
        let trace = {
            let mut exec = |c: &ToolCall| {
                let mut ws = Workspace { project: &mut p, built: &mut b, page: &mut page, selected: None, max_rows: 20, send_rows: true };
                tools::execute(&mut ws, c)
            };
            run.run(&[], "Add an average sale measure", &mut exec, &mut |_| {})
        };
        assert_eq!(trace.answer, "Added [Avg Sale].");
        assert!(p.model.measures.iter().any(|m| m.name == "Avg Sale"));
        let seen = prov.seen.lock().unwrap();
        assert!(seen[0].tools.is_empty());
        assert!(seen[0].messages[0].content.contains("TOOLS."));
        assert!(seen[1].messages.iter().any(|m| m.content.starts_with("Result of add_measure")));
    }

    #[test]
    fn provider_errors_end_the_run_cleanly() {
        let prov = ScriptedProvider::new(vec![], true);
        let cancel = AtomicBool::new(false);
        let run = AgentRun { provider: &prov, tools: vec![], context: String::new(), max_steps: 3, cancel: &cancel };
        let mut exec = |_: &ToolCall| ToolOutcome::default();
        let t = run.run(&[], "hi", &mut exec, &mut |_| {});
        assert!(!t.ok);
        assert_eq!(t.steps.last().map(|s| s.kind), Some(StepKind::Error));
    }
}
