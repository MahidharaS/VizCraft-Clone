//! The Copilot pane: chat with an agent about the open project. The agent runs on a background
//! thread; its tool calls come back here (the UI thread owns the project), and changes wait for
//! the user's "Apply" unless auto-apply is on. Every run is traced (trace viewer window).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use egui::{Color32, RichText, Ui};
use serde_json::json;
use vizcraft_ai::agent::{AgentEvent, AgentRun, StepKind, ToolExecutor, Trace, TraceStep};
use vizcraft_ai::brain::EventKind;
use vizcraft_ai::provider::{ChatProvider, ChatResponse, Message, ScriptedProvider, ToolCall};
use vizcraft_ai::tools::{self, ToolOutcome, Workspace};
use vizcraft_core::doc::*;

use crate::app::VizApp;
use crate::theme::Tokens;

pub enum Item {
    User(String),
    Assistant(String),
    Step(TraceStep),
    /// Index into `pending`… resolved approvals stay as notes.
    Approval(u64),
    Note(String, bool),
}

pub struct Pending {
    pub id: u64,
    pub call: ToolCall,
    pub desc: String,
    pub reply: Sender<ToolOutcome>,
}

enum Msg {
    Event(AgentEvent),
    Tool(ToolCall, Sender<ToolOutcome>),
}

#[derive(Default)]
pub struct Copilot {
    pub items: Vec<Item>,
    pub input: String,
    pub running: bool,
    rx: Option<Receiver<Msg>>,
    cancel: Option<Arc<AtomicBool>>,
    pub pending: Vec<Pending>,
    pub traces: Vec<Trace>,
    pub history: Vec<Message>,
    pub selected_trace: usize,
    pub expanded: std::collections::HashSet<usize>,
    next_id: u64,
    question: String,
}

/// Forwards tool calls to the UI thread and waits for the result.
struct ChannelExec {
    tx: Sender<Msg>,
    cancel: Arc<AtomicBool>,
}

impl ToolExecutor for ChannelExec {
    fn execute(&mut self, call: &ToolCall) -> ToolOutcome {
        let (rtx, rrx) = channel();
        if self.tx.send(Msg::Tool(call.clone(), rtx)).is_err() {
            return ToolOutcome { text: json!({"error": "The app closed."}).to_string(), summary: "app closed".into(), ..Default::default() };
        }
        loop {
            match rrx.recv_timeout(Duration::from_millis(200)) {
                Ok(o) => return o,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) if !self.cancel.load(Ordering::Relaxed) => continue,
                Err(_) => return ToolOutcome { text: json!({"error": "Cancelled by the user."}).to_string(), summary: "cancelled".into(), ..Default::default() },
            }
        }
    }
}

// ============================================================== running

/// Sends a question to the copilot.
pub fn ask(app: &mut VizApp, question: String) {
    let q = question.trim().to_string();
    if q.is_empty() || app.ai.copilot.running {
        return;
    }
    app.ai.copilot.items.push(Item::User(q.clone()));
    app.ai.copilot.question = q.clone();
    let provider: Box<dyn ChatProvider> = match app.ai.settings.active_provider() {
        Some(p) if app.ai.settings.has_llm() => p.build(),
        _ => Box::new(offline_plan(app, &q)),
    };
    let mut context = String::new();
    let profile = app.ai.brain.profile_summary();
    if !profile.is_empty() {
        context.push_str(&profile);
        context.push('\n');
    }
    if let Some(pg) = app.current_page() {
        context.push_str(&format!("The user is in the {} view{}; current report page \"{}\" (id {}).", crate::app::View::ALL.iter().find(|(v, _, _)| *v == app.view).map(|(_, _, l)| *l).unwrap_or("report"), if app.pq_open { " (Power Query Editor open)" } else { "" }, pg.name, pg.id));
    }
    if let Some(id) = app.selected.first() {
        context.push_str(&format!(" Selected element id: {id} (use 'selected')."));
    }
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let history = app.ai.copilot.history.clone();
    let max_steps = app.ai.settings.max_steps.max(2);
    let c2 = cancel.clone();
    std::thread::spawn(move || {
        let run = AgentRun { provider: provider.as_ref(), tools: tools::specs(), context, max_steps, cancel: &c2 };
        let mut exec = ChannelExec { tx: tx.clone(), cancel: c2.clone() };
        let tx2 = tx.clone();
        run.run(&history, &q, &mut exec, &mut |e| {
            let _ = tx2.send(Msg::Event(e));
        });
    });
    app.ai.copilot.rx = Some(rx);
    app.ai.copilot.cancel = Some(cancel);
    app.ai.copilot.running = true;
}

pub fn stop(app: &mut VizApp) {
    if let Some(c) = &app.ai.copilot.cancel {
        c.store(true, Ordering::Relaxed);
    }
    for p in app.ai.copilot.pending.drain(..) {
        let _ = p.reply.send(ToolOutcome { text: json!({"error": "Cancelled."}).to_string(), summary: "cancelled".into(), ..Default::default() });
    }
}

/// Runs a tool on the project (UI thread).
pub fn exec_tool(app: &mut VizApp, call: &ToolCall) -> ToolOutcome {
    let mutating = tools::is_mutating(&call.name);
    if mutating {
        app.checkpoint(&format!("Copilot: {}", tools::describe_call(call)));
    }
    let selected = app.selected.first().copied();
    let max_rows = app.ai.settings.privacy.max_rows_to_model;
    let send_rows = app.ai.settings.privacy.send_sample_rows || !app.ai.settings.has_llm() || app.ai.settings.active_provider().is_some_and(|p| p.kind.is_local());
    let out = {
        let mut ws = Workspace { project: &mut app.project, built: &mut app.built, page: &mut app.page, selected, max_rows, send_rows };
        tools::execute(&mut ws, call)
    };
    if out.mutated {
        crate::ai::after_ai_change(app);
        if !out.elements.is_empty() {
            app.selected = out.elements.clone();
        }
        if call.name == "create_visual" {
            for id in &out.elements {
                if let Some(v) = app.project.element(*id).and_then(|(_, e)| e.visual().cloned()) {
                    let fields = crate::ai::sigs_of(app, &v);
                    let kind = serde_json::to_value(v.kind).ok().and_then(|x| x.as_str().map(str::to_string)).unwrap_or_default();
                    crate::ai::track(app, EventKind::VisualCreated { kind, fields, source: "copilot".into() });
                }
            }
            app.view = crate::app::View::Report;
            app.pq_open = false;
        }
        if call.name == "add_measure" {
            let a = &call.arguments;
            let g = |k: &str| a.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
            crate::ai::track_measure(app, &g("name"), &g("dax"), &g("format"));
        }
    }
    for a in &out.actions {
        apply_action(app, a);
    }
    out
}

/// App-level requests from tools: navigation and commands.
pub fn apply_action(app: &mut VizApp, a: &serde_json::Value) {
    let g = |k: &str| a.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    match g("action").as_str() {
        "navigate" => {
            let v = g("view").to_lowercase();
            if v == "power-query" || v == "pq" || v == "transform" {
                app.pq_open = true;
            } else if let Some((view, _, _)) = crate::app::View::ALL.iter().find(|(_, k, label)| *k == v || label.to_lowercase().starts_with(&v)) {
                app.view = *view;
                app.pq_open = false;
            }
            let page = g("page");
            if !page.is_empty()
                && let Some(p) = app.project.report.pages.iter().find(|p| p.name.eq_ignore_ascii_case(&page) || p.id.to_string() == page)
            {
                app.page = p.id;
                app.view = crate::app::View::Report;
            }
        }
        "save" => app.save(false),
        "undo" => app.undo(),
        "redo" => app.redo(),
        "publish" => crate::publish::open(app, "service"),
        "reading-view" => app.report.reading = !app.report.reading,
        _ => {}
    }
}

pub fn poll(app: &mut VizApp, ctx: &egui::Context) {
    let Some(rx) = app.ai.copilot.rx.take() else { return };
    let mut keep = true;
    while let Ok(m) = rx.try_recv() {
        match m {
            Msg::Tool(call, reply) => {
                if tools::is_mutating(&call.name) && !app.ai.settings.privacy.auto_apply {
                    let id = app.ai.copilot.next_id;
                    app.ai.copilot.next_id += 1;
                    let desc = tools::describe_call(&call);
                    app.ai.copilot.pending.push(Pending { id, call, desc, reply });
                    app.ai.copilot.items.push(Item::Approval(id));
                } else {
                    let out = exec_tool(app, &call);
                    let _ = reply.send(out);
                }
            }
            Msg::Event(AgentEvent::Step(s)) => {
                if matches!(s.kind, StepKind::Tool | StepKind::Error) {
                    app.ai.copilot.items.push(Item::Step(s));
                }
            }
            Msg::Event(AgentEvent::Done(trace)) => {
                keep = false;
                let q = std::mem::take(&mut app.ai.copilot.question);
                app.ai.copilot.items.push(Item::Assistant(trace.answer.clone()));
                if trace.ok {
                    app.ai.copilot.history.push(Message::user(q.clone()));
                    app.ai.copilot.history.push(Message::assistant(trace.answer.clone()));
                }
                let tools_used: Vec<String> = trace.steps.iter().filter(|s| s.kind == StepKind::Tool && s.ok).map(|s| s.name.clone()).collect();
                let resolution = if tools_used.is_empty() { trace.answer.chars().take(160).collect() } else { format!("{} → {}", tools_used.join(", "), trace.answer.chars().take(120).collect::<String>()) };
                crate::ai::track(app, EventKind::Question { text: q, resolution, accepted: trace.ok });
                app.ai.copilot.traces.push(trace);
                app.ai.copilot.selected_trace = app.ai.copilot.traces.len().saturating_sub(1);
                app.ai.copilot.running = false;
            }
        }
    }
    if keep {
        app.ai.copilot.rx = Some(rx);
        ctx.request_repaint_after(Duration::from_millis(80));
    }
}

/// Without an LLM: a rule-based plan (Q&A + the brain's chart choice), run through the same
/// agent loop so it shows up in traces and asks before changing anything.
fn offline_plan(app: &mut VizApp, q: &str) -> ScriptedProvider {
    let l = q.to_lowercase();
    let say = |s: String| ChatResponse { content: s, ..Default::default() };
    let call = |name: &str, args: serde_json::Value| ChatResponse { tool_calls: vec![ToolCall { id: format!("offline_{name}"), name: name.into(), arguments: args }], ..Default::default() };
    let hint = "\n\nI'm in offline mode (rules + your personal brain). Connect a model in the AI hub — your own API key or a local model such as Ollama — for open-ended questions.";
    if ["explain", "summar", "describe", "why", "insight"].iter().any(|w| l.contains(w)) {
        let target = app.selected.first().copied().or_else(|| app.current_page().and_then(|p| p.elements.iter().find(|e| e.visual().is_some_and(|v| v.kind != VisualKind::Slicer)).map(|e| e.id)));
        if let Some(id) = target {
            let narrative = explain_offline(app, id);
            return ScriptedProvider::new(vec![call("explain_visual", json!({"element": id.to_string()})), say(format!("{narrative}{hint}"))], true);
        }
    }
    if ["model", "tables", "what data", "fields"].iter().any(|w| l.contains(w)) {
        let tables: Vec<String> = app.project.model.tables.iter().filter(|t| !t.hidden).map(|t| t.name.clone()).collect();
        return ScriptedProvider::new(
            vec![call("describe_model", json!({})), say(format!("The model has {} tables ({}) and {} measures.{hint}", tables.len(), tables.join(", "), app.project.model.measures.len()))],
            true,
        );
    }
    let enc = vizcraft_ai::formulate::rules_encoding(&app.project, &app.built, q);
    if !enc.y.is_empty() {
        let sigs = vizcraft_ai::formulate::signatures(&app.project, &app.built, &enc);
        let rec = app.ai.brain.recommend_chart(&sigs, 1);
        let kind = if enc.chart.is_empty() { rec.first().and_then(|s| if let vizcraft_ai::brain::SuggestionAction::ChartType { kind } = &s.action { Some(kind.clone()) } else { None }).unwrap_or_else(|| "clusteredColumn".into()) } else { enc.chart.clone() };
        let why = rec.first().map(|s| s.why.clone()).unwrap_or_default();
        let label = vizcraft_ai::brain::kind_label(&kind);
        let mut args = json!({"kind": kind, "values": enc.y, "title": q});
        if !enc.x.is_empty() {
            args["axis"] = json!(enc.x);
        }
        if !enc.color.is_empty() {
            args["legend"] = json!(enc.color);
        }
        return ScriptedProvider::new(vec![call("create_visual", args), say(format!("Made a {} for \"{q}\". {why}{hint}", label.to_lowercase()))], true);
    }
    ScriptedProvider::new(vec![say(format!("I couldn't map that to fields in this model. Try \"total sales by region\", \"explain the selected visual\" or \"what data is in the model\".{hint}"))], true)
}

/// A short narrative of a visual's data, computed locally.
fn explain_offline(app: &mut VizApp, id: u64) -> String {
    let call = ToolCall { id: "x".into(), name: "explain_visual".into(), arguments: json!({"element": id.to_string()}) };
    let out = {
        let mut page = app.page;
        let mut ws = Workspace { project: &mut app.project, built: &mut app.built, page: &mut page, selected: Some(id), max_rows: 5, send_rows: true };
        tools::execute(&mut ws, &call)
    };
    let v: serde_json::Value = serde_json::from_str(&out.text).unwrap_or_default();
    if !out.ok {
        return out.summary;
    }
    let mut s = format!("{} ({}, {} rows).", v["title"].as_str().unwrap_or("This visual"), v["kind"].as_str().unwrap_or(""), v["rowCount"]);
    for st in v["stats"].as_array().into_iter().flatten().take(2) {
        let name = st["value"].as_str().unwrap_or("value");
        let f = |x: &serde_json::Value| x.as_f64().map(|n| vizcraft_core::numfmt::format(n, "#,0.##")).unwrap_or_default();
        if st["maxAt"].as_str().is_some_and(|x| !x.is_empty()) {
            s.push_str(&format!("\n• {name}: highest for {} ({}), lowest for {} ({}); total {}.", st["maxAt"].as_str().unwrap_or(""), f(&st["max"]), st["minAt"].as_str().unwrap_or(""), f(&st["min"]), f(&st["sum"])));
        } else {
            s.push_str(&format!("\n• {name}: {}.", f(&st["sum"])));
        }
    }
    let filters: Vec<&str> = v["filters"].as_array().into_iter().flatten().filter_map(|x| x.as_str()).collect();
    if !filters.is_empty() {
        s.push_str(&format!("\nFiltered by: {}.", filters.join("; ")));
    }
    s
}

// ============================================================== pane

pub fn pane(app: &mut VizApp, ui: &mut Ui) {
    let t = app.tokens();
    let frame = egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::same(10)).stroke(egui::Stroke::new(1.0, t.border));
    egui::Panel::right("copilot-pane").default_size(360.0).size_range(300.0..=640.0).frame(frame).show(ui, |ui| {
        header(app, ui, &t);
        ui.add_space(4.0);
        // Input at the bottom.
        egui::Panel::bottom("copilot-input").frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin { left: 0, right: 0, top: 8, bottom: 0 })).show(ui, |ui| input(app, ui, &t));
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.panel)).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("copilot-scroll").stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if app.ai.copilot.items.is_empty() {
                    empty_state(app, ui, &t);
                } else {
                    items(app, ui, &t);
                }
                if app.ai.copilot.running && app.ai.copilot.pending.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0).color(t.accent));
                        ui.label(RichText::new("Working…").size(11.5).color(t.muted));
                    });
                }
            });
        });
    });
}

fn header(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
        crate::icons::paint(ui.painter(), r, "ai", t.accent);
        ui.label(RichText::new("Copilot").strong().size(14.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new("✕").frame(false)).on_hover_text("Close").clicked() {
                app.ai.copilot_open = false;
            }
            if ui.add(egui::Button::new(RichText::new("⚙").size(13.0)).frame(false)).on_hover_text("AI settings (providers, local models, privacy)").clicked() {
                app.view = crate::app::View::Ai;
            }
            if ui.add(egui::Button::new(RichText::new("Traces").size(11.5)).frame(false)).on_hover_text("Run traces: every model call and tool call, with latency and tokens").clicked() {
                app.ai.show_traces = !app.ai.show_traces;
            }
            if ui.add(egui::Button::new(RichText::new("New chat").size(11.5)).frame(false)).clicked() && !app.ai.copilot.running {
                app.ai.copilot.items.clear();
                app.ai.copilot.history.clear();
            }
        });
    });
    let mode = app.ai.mode_label();
    let on = app.ai.settings.has_llm();
    ui.horizontal(|ui| {
        crate::ai::pill(ui, t, &mode, on).on_hover_text(if on { "Answers come from this model. Change it in the AI hub." } else { "No model connected: questions are answered by rules over your model and your personal brain." });
    });
    ui.separator();
}

fn empty_state(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    ui.add_space(12.0);
    ui.label(RichText::new("Ask about your data").strong().size(15.0));
    ui.label(RichText::new("The copilot can read your model, run DAX and M, build visuals, write measures and explain numbers. It asks before changing the report; every change can be undone.").size(11.5).color(t.muted));
    ui.add_space(10.0);
    let examples = ["Show total sales by region", "Explain the selected visual", "Add a profit margin measure", "Which product had the highest sales?", "What data is in the model?"];
    for ex in examples {
        let r = egui::Frame::new().fill(t.panel_alt).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6.0).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(ex).size(12.0));
        });
        if r.response.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            ask(app, ex.to_string());
        }
        ui.add_space(4.0);
    }
    let profile = app.ai.brain.model.events;
    if profile > 0 {
        ui.add_space(8.0);
        ui.label(RichText::new(format!("Personalised by your brain: {profile} actions learned on this device.")).size(10.5).color(t.muted));
    }
}

fn items(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let mut decide: Option<(u64, bool)> = None;
    let n = app.ai.copilot.items.len();
    for i in 0..n {
        let Some(item) = app.ai.copilot.items.get(i) else { continue };
        match item {
            Item::User(text) => {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    egui::Frame::new().fill(t.accent).corner_radius(10.0).inner_margin(egui::Margin::symmetric(10, 7)).show(ui, |ui| {
                        ui.set_max_width(ui.available_width() * 0.85);
                        ui.label(RichText::new(text).color(Color32::WHITE).size(12.5));
                    });
                });
                ui.add_space(6.0);
            }
            Item::Assistant(text) => {
                egui::Frame::new().fill(t.panel_alt).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(10.0).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for line in text.lines() {
                        let l = line.trim_start();
                        if let Some(h) = l.strip_prefix("## ").or_else(|| l.strip_prefix("# ")) {
                            ui.label(RichText::new(h).strong());
                        } else if let Some(b) = l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")) {
                            ui.label(RichText::new(format!("•  {}", b.replace("**", ""))).size(12.5));
                        } else if l.starts_with("```") {
                        } else {
                            ui.label(RichText::new(line.replace("**", "")).size(12.5));
                        }
                    }
                });
                ui.add_space(8.0);
            }
            Item::Step(s) => {
                let open = app.ai.copilot.expanded.contains(&i);
                let resp = ui
                    .horizontal(|ui| {
                        let (mark, col) = if s.ok { ("✓", t.good) } else { ("✕", t.danger) };
                        ui.label(RichText::new(mark).color(col).size(11.0));
                        ui.label(RichText::new(tool_label(&s.name)).size(11.5).strong().color(t.text));
                        let preview: String = summary_of(&s.output);
                        ui.label(RichText::new(preview).size(11.0).color(t.muted));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(format!("{:.0} ms", s.latency_ms)).size(10.5).color(t.muted));
                        });
                    })
                    .response
                    .interact(egui::Sense::click());
                if resp.on_hover_text("Show input and output").clicked() {
                    if open {
                        app.ai.copilot.expanded.remove(&i);
                    } else {
                        app.ai.copilot.expanded.insert(i);
                    }
                }
                if open {
                    code_block(ui, t, &format!("in:  {}\nout: {}", s.input, s.output.chars().take(2000).collect::<String>()));
                }
            }
            Item::Approval(id) => {
                let id = *id;
                let pending = app.ai.copilot.pending.iter().find(|p| p.id == id);
                egui::Frame::new().fill(t.panel).stroke(egui::Stroke::new(1.0, t.accent)).corner_radius(8.0).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    match pending {
                        Some(p) => {
                            ui.label(RichText::new("Copilot wants to change the report").size(11.0).color(t.muted));
                            ui.label(RichText::new(&p.desc).size(12.5).strong());
                            ui.horizontal(|ui| {
                                if ui.add(egui::Button::new(RichText::new("Apply").color(Color32::WHITE)).fill(t.accent)).clicked() {
                                    decide = Some((id, true));
                                }
                                if ui.button("Skip").clicked() {
                                    decide = Some((id, false));
                                }
                                ui.label(RichText::new("Undo with Ctrl+Z").size(10.5).color(t.muted));
                            });
                        }
                        None => {
                            ui.label(RichText::new("Change handled").size(11.0).color(t.muted));
                        }
                    }
                });
                ui.add_space(6.0);
            }
            Item::Note(text, err) => {
                ui.label(RichText::new(text).size(11.5).color(if *err { t.danger } else { t.muted }));
            }
        }
    }
    if let Some((id, apply)) = decide
        && let Some(pos) = app.ai.copilot.pending.iter().position(|p| p.id == id)
    {
        let p = app.ai.copilot.pending.remove(pos);
        let out = if apply {
            exec_tool(app, &p.call)
        } else {
            ToolOutcome { text: json!({"error": "The user declined this change."}).to_string(), summary: "declined".into(), ..Default::default() }
        };
        crate::ai::track(app, EventKind::Suggestion { arm: format!("copilot:{}", p.call.name), id: p.desc.clone(), accepted: apply });
        app.ai.copilot.items.push(Item::Note(if apply { format!("Applied: {}", p.desc) } else { format!("Skipped: {}", p.desc) }, false));
        let _ = p.reply.send(out);
    }
}

fn tool_label(name: &str) -> String {
    match name {
        "describe_model" => "Read the model".into(),
        "run_dax" => "Ran DAX".into(),
        "run_m" => "Ran M".into(),
        "preview_table" => "Previewed a table".into(),
        "create_visual" => "Created a visual".into(),
        "set_format" => "Formatted".into(),
        "add_measure" => "Added a measure".into(),
        "add_filter" => "Added a filter".into(),
        "create_page" => "Created a page".into(),
        "explain_visual" => "Read a visual".into(),
        "create_query" => "Created a query".into(),
        "suggest_relationships" => "Checked relationships".into(),
        "model" | "cancelled" => "Model error".into(),
        n => n.to_string(),
    }
}

fn summary_of(output: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(output).unwrap_or_default();
    let s = if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
        e.to_string()
    } else if let Some(n) = v.get("rowCount") {
        format!("{n} rows")
    } else if let Some(c) = v.get("created") {
        format!("#{c}")
    } else if let Some(m) = v.get("measure").and_then(|m| m.as_str()) {
        format!("[{m}]")
    } else if v.is_null() {
        output.to_string()
    } else {
        String::new()
    };
    s.chars().take(60).collect()
}

fn code_block(ui: &mut Ui, t: &Tokens, text: &str) {
    egui::Frame::new().fill(t.panel_alt).corner_radius(4.0).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(text).monospace().size(10.5).color(t.text));
    });
}

fn input(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let running = app.ai.copilot.running;
    let mut send = false;
    let r = ui.add(
        egui::TextEdit::multiline(&mut app.ai.copilot.input)
            .hint_text("Ask about your data, or ask for a visual, a measure, a filter…")
            .desired_rows(2)
            .desired_width(f32::INFINITY)
            .id_salt("copilot-input-box"),
    );
    if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift) {
        send = true;
    }
    ui.horizontal(|ui| {
        let mut auto = app.ai.settings.privacy.auto_apply;
        if ui.checkbox(&mut auto, RichText::new("Apply changes without asking").size(11.0)).changed() {
            app.ai.settings.privacy.auto_apply = auto;
            let _ = app.ai.settings.save();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if running {
                if ui.button("Stop").clicked() {
                    stop(app);
                }
            } else if ui.add(egui::Button::new(RichText::new("Send").color(Color32::WHITE)).fill(t.accent)).clicked() {
                send = true;
            }
        });
    });
    if send && !running {
        let q = std::mem::take(&mut app.ai.copilot.input);
        ask(app, q.trim_end_matches('\n').to_string());
    }
}

// ============================================================== traces

/// A LangSmith-style trace viewer: runs, and each run's steps as a timeline.
pub fn trace_window(app: &mut VizApp, ctx: &egui::Context) {
    if !app.ai.show_traces {
        return;
    }
    let t = app.tokens();
    let mut open = true;
    egui::Window::new("Copilot traces").open(&mut open).default_size([760.0, 460.0]).show(ctx, |ui| {
        if app.ai.copilot.traces.is_empty() {
            ui.label(RichText::new("No runs yet. Ask the copilot something; each run's model calls and tool calls show here with inputs, outputs, latency and tokens.").color(t.muted));
            return;
        }
        egui::Panel::left("trace-runs").default_size(220.0).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("runs").show(ui, |ui| {
                for (i, tr) in app.ai.copilot.traces.iter().enumerate().rev() {
                    let sel = app.ai.copilot.selected_trace == i;
                    let r = ui.add(egui::Button::selectable(sel, RichText::new(format!("{}\n{:.1} s · {} steps · {} tok", tr.question.chars().take(34).collect::<String>(), tr.total_ms / 1000.0, tr.steps.len(), tr.tokens.input_tokens + tr.tokens.output_tokens)).size(11.0)));
                    if r.clicked() {
                        app.ai.copilot.selected_trace = i;
                    }
                }
            });
        });
        let Some(tr) = app.ai.copilot.traces.get(app.ai.copilot.selected_trace).cloned() else { return };
        ui.horizontal(|ui| {
            ui.label(RichText::new(&tr.question).strong());
            ui.label(RichText::new(format!("{} · {:.0} ms · in {} / out {} tokens", tr.provider, tr.total_ms, tr.tokens.input_tokens, tr.tokens.output_tokens)).size(11.0).color(t.muted));
        });
        ui.separator();
        let max_ms = tr.steps.iter().map(|s| s.latency_ms).fold(1.0, f64::max);
        egui::ScrollArea::vertical().id_salt("steps").show(ui, |ui| {
            for (i, s) in tr.steps.iter().enumerate() {
                let (tag, col) = match s.kind {
                    StepKind::Model => ("LLM", t.accent),
                    StepKind::Tool => ("TOOL", t.good),
                    StepKind::Answer => ("OUT", t.muted),
                    StepKind::Error => ("ERR", t.danger),
                };
                egui::CollapsingHeader::new(RichText::new(format!("{tag}  {}", s.name)).color(col).size(12.0)).id_salt(("trace-step", tr.id.clone(), i)).show(ui, |ui| {
                    ui.label(RichText::new(format!("{:.1} ms · in {} / out {} tokens", s.latency_ms, s.tokens.input_tokens, s.tokens.output_tokens)).size(10.5).color(t.muted));
                    if !s.input.is_empty() {
                        ui.label(RichText::new("Input").size(10.5).strong());
                        code_block(ui, &t, &s.input.chars().take(4000).collect::<String>());
                    }
                    ui.label(RichText::new("Output").size(10.5).strong());
                    code_block(ui, &t, &s.output.chars().take(4000).collect::<String>());
                });
                // Latency bar.
                let w = ui.available_width();
                let (r, _) = ui.allocate_exact_size(egui::vec2(w, 4.0), egui::Sense::hover());
                let frac = (s.latency_ms / max_ms).clamp(0.02, 1.0) as f32;
                ui.painter().rect_filled(egui::Rect::from_min_size(r.min, egui::vec2(w * frac, 3.0)), 1.5, col.gamma_multiply(0.6));
            }
        });
    });
    if !open {
        app.ai.show_traces = false;
    }
}
