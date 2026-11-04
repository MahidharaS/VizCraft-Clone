//! The AI hub: models and keys (bring your own key or a local model), local server detection,
//! the OpenJEV decision model, the personal brain (what it learned, export, fine-tune, forget)
//! and privacy.

use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use egui::{Color32, RichText, Ui};
use vizcraft_ai::decide::{Decide, Decision, LlmDecider, OpenJevDecider, Question, decide_with_fallback};
use vizcraft_ai::detect::{self, Detected, ServerKind};
use vizcraft_ai::installed::{self, Installed};
use vizcraft_ai::provider::{ChatRequest, Message, ProviderKind};
use vizcraft_ai::settings::DecisionBackend;

use crate::app::VizApp;
use crate::theme::Tokens;

#[derive(Default)]
pub struct HubUi {
    pub tab: usize,
    pub editing: String,
    pub show_key: bool,
    scan: Option<Receiver<Vec<Detected>>>,
    pub detected: Option<Vec<Detected>>,
    pub scanning: bool,
    /// Models installed on disk (Ollama store, LM Studio, GGUF files, Hugging Face cache).
    pub installed: Option<Vec<Installed>>,
    inst_scan: Option<Receiver<Vec<Installed>>>,
    /// A server being started: (label, deadline, base url, model to select).
    starting: Option<(String, Instant, String, Installed)>,
    pub start_error: Option<String>,
    /// The startup scan ran (auto-selects a local model when nothing is configured).
    pub auto_scanned: bool,
    auto_pick: bool,
    test: Option<Receiver<Result<(String, f64), String>>>,
    pub test_result: Option<Result<(String, f64), String>>,
    decision: Option<Receiver<(Decision, Option<String>)>>,
    pub decision_result: Option<(Decision, Option<String>)>,
    pub confirm_forget: bool,
    pub dirty: bool,
}

pub fn poll(app: &mut VizApp, ctx: &egui::Context) {
    // At startup, look for local models in the background; with no model configured, pick a
    // running local one so Copilot works out of the box.
    if !app.ai.hub.auto_scanned {
        app.ai.hub.auto_scanned = true;
        app.ai.hub.auto_pick = !app.ai.settings.has_llm();
        start_scan(app);
    }
    if let Some(rx) = &app.ai.hub.inst_scan
        && let Ok(v) = rx.try_recv()
    {
        app.ai.hub.installed = Some(v);
        app.ai.hub.inst_scan = None;
    }
    // A server we started: rescan until it answers (or give up after the deadline).
    if let Some((label, deadline, base, model)) = app.ai.hub.starting.clone() {
        let up = app.ai.hub.detected.iter().flatten().any(|d| d.probe.base_url == base);
        if up {
            app.ai.hub.starting = None;
            let kind = match model.runtime {
                "Ollama" => ProviderKind::Ollama,
                "LM Studio" => ProviderKind::LmStudio,
                _ => ProviderKind::LlamaCpp,
            };
            let name = if model.runtime == "Ollama" { Some(model.name.clone()) } else { app.ai.hub.detected.iter().flatten().find(|d| d.probe.base_url == base).and_then(|d| d.models.first().cloned()) };
            use_local(app, kind, &base, name);
            app.toast(format!("{label} is running"));
        } else if Instant::now() > deadline {
            app.ai.hub.starting = None;
            app.ai.hub.start_error = Some(format!("{label} didn't answer on {base} within 30 s. Start it in a terminal: {}", installed::serve_command(&model).unwrap_or_default()));
        } else if !app.ai.hub.scanning {
            start_scan(app);
        }
        ctx.request_repaint_after(Duration::from_millis(400));
    }
    if app.ai.hub.auto_pick && !app.ai.hub.scanning && app.ai.hub.detected.is_some() {
        app.ai.hub.auto_pick = false;
        if !app.ai.settings.has_llm()
            && let Some(d) = app.ai.hub.detected.iter().flatten().find(|d| matches!(d.probe.kind, ServerKind::Llm(_)) && !d.models.is_empty()).cloned()
            && let ServerKind::Llm(kind) = d.probe.kind
        {
            use_local(app, kind, &d.probe.base_url, d.models.first().cloned());
        }
    }
    let h = &mut app.ai.hub;
    let mut busy = false;
    if let Some(rx) = &h.scan {
        match rx.try_recv() {
            Ok(v) => {
                h.detected = Some(v);
                h.scan = None;
                h.scanning = false;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => busy = true,
            Err(_) => {
                h.scan = None;
                h.scanning = false;
            }
        }
    }
    if let Some(rx) = &h.test {
        match rx.try_recv() {
            Ok(v) => {
                h.test_result = Some(v);
                h.test = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => busy = true,
            Err(_) => h.test = None,
        }
    }
    if let Some(rx) = &h.decision {
        match rx.try_recv() {
            Ok(v) => {
                h.decision_result = Some(v);
                h.decision = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => busy = true,
            Err(_) => h.decision = None,
        }
    }
    if h.dirty {
        h.dirty = false;
        if let Err(e) = app.ai.settings.save() {
            app.error(format!("Couldn't save AI settings: {e}"));
        }
    }
    if busy {
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

/// First-run notice (inline banner in AI surfaces, never a blocking dialog).
pub fn disclosure(_app: &mut VizApp, _ctx: &egui::Context) {}

pub fn banner(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    if app.ai.settings.privacy.disclosure_seen {
        return;
    }
    egui::Frame::new().fill(t.accent_soft).corner_radius(6.0).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("VizCraft learns how you work — on this device").strong().size(12.0));
        ui.label(
            RichText::new("Your personal brain records actions like the visuals you build and the formatting you choose in ~/.vizcraft/brain, to recommend what you usually do. Nothing is uploaded. You can export it, switch it off or forget everything in the AI hub.")
                .size(11.0)
                .color(t.text),
        );
        ui.horizontal(|ui| {
            if ui.button("Got it").clicked() {
                app.ai.settings.privacy.disclosure_seen = true;
                app.ai.hub.dirty = true;
            }
            if ui.button("Turn learning off").clicked() {
                app.ai.settings.privacy.disclosure_seen = true;
                app.ai.settings.privacy.learn = false;
                app.ai.brain.enabled = false;
                app.ai.hub.dirty = true;
            }
        });
    });
    ui.add_space(6.0);
}

const TABS: [&str; 5] = ["Models & keys", "Local models", "Decisions (OpenJEV)", "Personal brain", "Privacy"];

/// The AI hub view (rail key "ai").
pub fn show(app: &mut VizApp, ui: &mut Ui) {
    let t = app.tokens();
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg).inner_margin(egui::Margin::symmetric(28, 18))).show(ui, |ui| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
            crate::icons::paint(ui.painter(), r, "ai", t.accent);
            ui.vertical(|ui| {
                ui.label(RichText::new("AI hub").strong().size(20.0));
                ui.label(RichText::new("Bring your own API key or run a model on your machine. The personal brain learns from you, on device.").size(12.0).color(t.muted));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                crate::ai::pill(ui, &t, &app.ai.mode_label(), app.ai.settings.has_llm());
            });
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            for (i, label) in TABS.iter().enumerate() {
                let on = app.ai.hub.tab == i;
                let r = ui.add(egui::Button::new(RichText::new(*label).size(12.5).color(if on { t.accent } else { t.text }).strong()).frame(false));
                if on {
                    ui.painter().line_segment([r.rect.left_bottom(), r.rect.right_bottom()], egui::Stroke::new(2.0, t.accent));
                }
                if r.clicked() {
                    app.ai.hub.tab = i;
                }
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().id_salt("ai-hub").auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(980.0);
            banner(app, ui, &t);
            match app.ai.hub.tab {
                1 => local_tab(app, ui, &t),
                2 => decisions_tab(app, ui, &t),
                3 => brain_tab(app, ui, &t),
                4 => privacy_tab(app, ui, &t),
                _ => models_tab(app, ui, &t),
            }
        });
    });
}

fn card(ui: &mut Ui, t: &Tokens, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new().fill(t.panel).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8.0).inner_margin(egui::Margin::same(14)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
    ui.add_space(10.0);
}

fn h2(ui: &mut Ui, s: &str) {
    ui.label(RichText::new(s).strong().size(14.0));
    ui.add_space(4.0);
}

fn code(ui: &mut Ui, t: &Tokens, s: &str) {
    egui::Frame::new().fill(t.panel_alt).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(4.0).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(s).monospace().size(11.0));
    });
}

// ============================================================== models

fn models_tab(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    if app.ai.hub.editing.is_empty() {
        app.ai.hub.editing = if app.ai.settings.active.is_empty() { "anthropic".into() } else { app.ai.settings.active.clone() };
    }
    ui.columns(2, |cols| {
        let [left, right] = cols else { return };
        // Provider list.
        card(left, t, |ui| {
            h2(ui, "Providers");
            let none_on = app.ai.settings.active.is_empty();
            if ui.radio(none_on, RichText::new("None — offline (rules + personal brain)").size(12.0)).clicked() {
                app.ai.settings.active.clear();
                app.ai.hub.dirty = true;
            }
            ui.add_space(4.0);
            let mut last_group = "";
            for p in app.ai.settings.providers.clone() {
                let group = if p.kind.is_local() { "On this machine" } else if p.kind == ProviderKind::Custom { "Other" } else { "Cloud (your key)" };
                if group != last_group {
                    ui.add_space(4.0);
                    ui.label(RichText::new(group).size(10.5).color(t.muted));
                    last_group = group;
                }
                ui.horizontal(|ui| {
                    let active = app.ai.settings.active == p.id;
                    if ui.radio(active, "").on_hover_text("Use this provider").clicked() {
                        app.ai.settings.active = p.id.clone();
                        app.ai.hub.editing = p.id.clone();
                        app.ai.hub.dirty = true;
                    }
                    let sel = app.ai.hub.editing == p.id;
                    if ui.add(egui::Button::selectable(sel, RichText::new(&p.label).size(12.0))).clicked() {
                        app.ai.hub.editing = p.id.clone();
                        app.ai.hub.test_result = None;
                    }
                    let src = p.key_source();
                    let col = match src {
                        "missing" => t.danger,
                        "not needed" => t.muted,
                        _ => t.good,
                    };
                    ui.label(RichText::new(if src == "missing" { "no key" } else if src == "not needed" { "local" } else { "key set" }).size(10.5).color(col));
                });
            }
        });
        // Editor.
        let id = app.ai.hub.editing.clone();
        let detected_models: Vec<String> = app.ai.hub.detected.iter().flatten().flat_map(|d| d.models.clone()).collect();
        card(right, t, |ui| {
            let Some(p) = app.ai.settings.provider_mut(&id) else { return };
            h2(ui, &p.label.clone());
            let mut changed = false;
            egui::Grid::new("prov-grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label("Model");
                ui.horizontal(|ui| {
                    changed |= ui.add(egui::TextEdit::singleline(&mut p.model).desired_width(220.0)).changed();
                    if !detected_models.is_empty() && p.kind.is_local() {
                        egui::ComboBox::from_id_salt("det-models").selected_text("Detected").width(90.0).show_ui(ui, |ui| {
                            for m in &detected_models {
                                if ui.selectable_label(p.model == *m, m).clicked() {
                                    p.model = m.clone();
                                    changed = true;
                                }
                            }
                        });
                    }
                });
                ui.end_row();
                ui.label("Endpoint");
                changed |= ui.add(egui::TextEdit::singleline(&mut p.base_url).desired_width(300.0)).changed();
                ui.end_row();
                if p.kind.needs_key() || p.kind == ProviderKind::Custom {
                    ui.label("API key");
                    ui.horizontal(|ui| {
                        changed |= ui.add(egui::TextEdit::singleline(&mut p.api_key).password(!app.ai.hub.show_key).desired_width(240.0).hint_text(p.kind.env_key().map(|k| format!("or set {k}")).unwrap_or_default())).changed();
                        if ui.small_button(if app.ai.hub.show_key { "Hide" } else { "Show" }).clicked() {
                            app.ai.hub.show_key = !app.ai.hub.show_key;
                        }
                    });
                    ui.end_row();
                    ui.label("");
                    ui.label(RichText::new(format!("Key: {}", p.key_source())).size(11.0).color(t.muted));
                    ui.end_row();
                }
                if p.kind == ProviderKind::AzureOpenAi {
                    ui.label("API version");
                    changed |= ui.text_edit_singleline(&mut p.api_version).changed();
                    ui.end_row();
                }
                ui.label("Tool calling");
                changed |= ui.checkbox(&mut p.native_tools, "Native function calling (off: JSON tool protocol for small local models)").changed();
                ui.end_row();
                ui.label("Timeout");
                changed |= ui.add(egui::DragValue::new(&mut p.timeout_secs).range(5..=1800).suffix(" s")).changed();
                ui.end_row();
            });
            if changed {
                app.ai.hub.dirty = true;
            }
            let cfg = p.clone();
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let testing = app.ai.hub.test.is_some();
                if ui.add_enabled(!testing, egui::Button::new("Test connection")).clicked() {
                    let (tx, rx) = channel();
                    std::thread::spawn(move || {
                        let t0 = Instant::now();
                        let req = ChatRequest { messages: vec![Message::user("Reply with the single word: ready")], tools: vec![], temperature: Some(0.0), max_tokens: 20, json_mode: false };
                        let r = cfg.build().chat(&req).map(|r| (r.content.trim().chars().take(80).collect::<String>(), t0.elapsed().as_secs_f64() * 1000.0)).map_err(|e| e.to_string());
                        let _ = tx.send(r);
                    });
                    app.ai.hub.test = Some(rx);
                    app.ai.hub.test_result = None;
                }
                if testing {
                    ui.add(egui::Spinner::new().size(14.0));
                }
                if app.ai.settings.active != id && ui.button("Use this provider").clicked() {
                    app.ai.settings.active = id.clone();
                    app.ai.hub.dirty = true;
                }
            });
            match &app.ai.hub.test_result {
                Some(Ok((answer, ms))) => {
                    ui.label(RichText::new(format!("Connected in {ms:.0} ms — the model said \"{answer}\"")).color(t.good).size(12.0));
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(e).color(t.danger).size(12.0));
                }
                None => {}
            }
        });
        card(right, t, |ui| {
            h2(ui, "Where keys are kept");
            ui.label(
                RichText::new("Keys typed here are saved in plain text in ~/.vizcraft/ai.json (readable only by your user account), never in project files. Environment variables (OPENAI_API_KEY, ANTHROPIC_API_KEY, GEMINI_API_KEY, HF_TOKEN, GROQ_API_KEY, OPENROUTER_API_KEY, TOGETHER_API_KEY) are used when no key is saved — prefer them on shared machines.")
                    .size(11.5)
                    .color(t.muted),
            );
        });
    });
}

// ============================================================== local

fn local_tab(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    card(ui, t, |ui| {
        ui.horizontal(|ui| {
            h2(ui, "Models running on this machine");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(!app.ai.hub.scanning, egui::Button::new("Scan")).on_hover_text("Probe Ollama (11434), LM Studio (1234), llama.cpp (8080), vLLM (8000), MLX (8081), Jan (1337), GPT4All (4891), OpenJEV (3000, 8765) and list models installed on disk").clicked() {
                    start_scan(app);
                }
                if app.ai.hub.scanning {
                    ui.add(egui::Spinner::new().size(14.0));
                }
            });
        });
        match &app.ai.hub.detected {
            None => {
                ui.label(RichText::new("Scan to find local model servers.").color(t.muted));
            }
            Some(v) if v.is_empty() => {
                ui.label(RichText::new("No local server answered. Start one (below) and scan again.").color(t.muted));
            }
            Some(v) => {
                for d in v.clone() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("●").color(t.good));
                        ui.label(RichText::new(d.probe.label).strong());
                        ui.label(RichText::new(&d.probe.base_url).size(11.0).color(t.muted));
                        ui.label(RichText::new(format!("{} model(s)", d.models.len())).size(11.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match d.probe.kind {
                            ServerKind::Llm(kind) => {
                                if ui.button("Use").clicked() {
                                    use_local(app, kind, &d.probe.base_url, d.models.first().cloned());
                                }
                            }
                            ServerKind::OpenJev => {
                                if ui.button("Use for decisions").clicked() {
                                    app.ai.settings.openjev.base_url = d.probe.base_url.clone();
                                    app.ai.settings.decision = DecisionBackend::OpenJev;
                                    app.ai.hub.dirty = true;
                                }
                            }
                        });
                    });
                    if !d.models.is_empty() {
                        ui.label(RichText::new(d.models.iter().take(8).cloned().collect::<Vec<_>>().join(", ")).size(11.0).color(t.muted));
                    }
                    ui.add_space(4.0);
                }
            }
        }
    });
    if app.ai.hub.detected.is_none() && !app.ai.hub.scanning {
        start_scan(app);
    }
    card(ui, t, |ui| {
        ui.horizontal(|ui| {
            h2(ui, "Installed on this computer");
            if app.ai.hub.inst_scan.is_some() {
                ui.add(egui::Spinner::new().size(14.0));
            }
            if let Some((label, ..)) = &app.ai.hub.starting {
                ui.add(egui::Spinner::new().size(14.0));
                ui.label(RichText::new(format!("starting {label}…")).size(11.0).color(t.muted));
            }
        });
        ui.label(RichText::new("Models found on disk: Ollama (~/.ollama), LM Studio (~/.lmstudio), GGUF files (llama.cpp) and the Hugging Face cache, including MLX. Start & use launches the right server for you.").size(11.0).color(t.muted));
        if let Some(e) = app.ai.hub.start_error.clone() {
            ui.label(RichText::new(e).size(11.0).color(t.danger));
        }
        let list = app.ai.hub.installed.clone().unwrap_or_default();
        if list.is_empty() && app.ai.hub.installed.is_some() {
            ui.label(RichText::new("No models found on disk. Install one, e.g. `ollama pull qwen2.5:7b-instruct`, then Scan.").color(t.muted));
        }
        let running: Vec<Detected> = app.ai.hub.detected.clone().unwrap_or_default();
        let mut action: Option<(Installed, bool)> = None;
        egui::ScrollArea::vertical().id_salt("ai-installed").max_height(260.0).show(ui, |ui| {
            egui::Grid::new("ai-inst-grid").striped(true).spacing([14.0, 4.0]).show(ui, |ui| {
                for m in &list {
                    ui.label(RichText::new(m.runtime).size(11.0).color(t.muted));
                    ui.label(RichText::new(&m.name).size(11.5));
                    ui.label(RichText::new(m.size_label()).size(11.0).color(t.muted));
                    let server = running.iter().find(|d| match m.runtime {
                        "Ollama" => d.probe.label == "Ollama",
                        "LM Studio" => d.probe.label == "LM Studio",
                        "MLX" => d.probe.label == "MLX server",
                        _ => d.probe.label == "llama.cpp server",
                    });
                    let loaded = server.is_some_and(|d| m.runtime != "Ollama" || d.models.iter().any(|x| x == &m.name));
                    match (server.is_some(), m.runtime) {
                        (_, "Hugging Face") => {
                            ui.label(RichText::new("serve with vLLM or convert to GGUF").size(10.5).color(t.muted));
                        }
                        (true, _) if loaded || m.runtime == "Ollama" => {
                            if ui.button("Use").clicked() {
                                action = Some((m.clone(), false));
                            }
                        }
                        _ => {
                            if ui.button("Start & use").on_hover_text(installed::serve_command(m).unwrap_or_default()).clicked() {
                                action = Some((m.clone(), true));
                            }
                        }
                    }
                    ui.end_row();
                }
            });
        });
        if let Some((m, start)) = action {
            if start {
                start_and_use(app, &m);
            } else {
                let (kind, base) = match m.runtime {
                    "Ollama" => (ProviderKind::Ollama, running.iter().find(|d| d.probe.label == "Ollama").map(|d| d.probe.base_url.clone()).unwrap_or_default()),
                    "LM Studio" => (ProviderKind::LmStudio, "http://127.0.0.1:1234/v1".into()),
                    "MLX" => (ProviderKind::LlamaCpp, "http://127.0.0.1:8081/v1".into()),
                    _ => (ProviderKind::LlamaCpp, "http://127.0.0.1:8080/v1".into()),
                };
                let model = if m.runtime == "Ollama" { Some(m.name.clone()) } else { running.iter().find(|d| d.probe.base_url == base).and_then(|d| d.models.first().cloned()) };
                use_local(app, kind, &base, model);
            }
        }
        let tools: Vec<String> = ["ollama", "lms", "llama-server", "mlx_lm.server", "vllm"].iter().map(|b| format!("{b} {}", if installed::find_bin(b).is_some() { "✓" } else { "–" })).collect();
        ui.label(RichText::new(format!("Command-line tools: {}", tools.join("  ·  "))).size(10.5).color(t.muted));
    });
    ui.columns(2, |cols| {
        let [a, b] = cols else { return };
        card(a, t, |ui| {
            h2(ui, "Ollama");
            ui.label(RichText::new("The easiest local option. Models with tool calling work best (Qwen 2.5, Llama 3.1+).").size(11.5).color(t.muted));
            code(ui, t, "ollama pull qwen2.5:7b-instruct\nollama serve        # http://localhost:11434");
        });
        card(a, t, |ui| {
            h2(ui, "llama.cpp server");
            code(ui, t, "llama-server -m qwen2.5-7b-instruct-q4_k_m.gguf \\\n  --port 8080 --jinja   # OpenAI-compatible /v1");
            ui.label(RichText::new("Without native tool support, VizCraft uses its JSON tool protocol.").size(11.0).color(t.muted));
        });
        card(b, t, |ui| {
            h2(ui, "LM Studio");
            ui.label(RichText::new("Load a model, open the Developer tab and start the server (port 1234).").size(11.5).color(t.muted));
        });
        card(b, t, |ui| {
            h2(ui, "vLLM (GPU)");
            code(ui, t, "vllm serve Qwen/Qwen2.5-7B-Instruct --port 8000 \\\n  --enable-auto-tool-choice --tool-call-parser hermes");
        });
    });
}

fn start_scan(app: &mut VizApp) {
    let (tx, rx) = channel();
    let token = app.ai.settings.openjev.token.clone();
    std::thread::spawn(move || {
        let _ = tx.send(detect::detect(&detect::default_probes(), Duration::from_millis(900), &token));
    });
    app.ai.hub.scan = Some(rx);
    app.ai.hub.scanning = true;
    if app.ai.hub.inst_scan.is_none() && app.ai.hub.starting.is_none() {
        let (itx, irx) = channel();
        std::thread::spawn(move || {
            let _ = itx.send(installed::scan());
        });
        app.ai.hub.inst_scan = Some(irx);
    }
}

/// Starts the server for an installed model and selects it once it answers.
fn start_and_use(app: &mut VizApp, m: &Installed) {
    app.ai.hub.start_error = None;
    match installed::start_server(m) {
        Ok(base) => {
            app.ai.hub.starting = Some((m.runtime.to_string(), Instant::now() + Duration::from_secs(30), base, m.clone()));
            app.toast(format!("Starting {}…", m.runtime));
        }
        Err(e) => app.ai.hub.start_error = Some(e),
    }
}

fn use_local(app: &mut VizApp, kind: ProviderKind, base: &str, model: Option<String>) {
    let id = {
        let Some(p) = app.ai.settings.providers.iter_mut().find(|p| p.kind == kind) else { return };
        p.base_url = base.to_string();
        if let Some(m) = model {
            p.model = m;
        }
        p.id.clone()
    };
    app.ai.settings.active = id.clone();
    app.ai.hub.editing = id;
    app.ai.hub.dirty = true;
    app.toast("Local model selected");
}

// ============================================================== decisions

fn decisions_tab(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    card(ui, t, |ui| {
        h2(ui, "Decision backend");
        ui.label(
            RichText::new("Fast typed decisions inside the app — which chart fits, whether a column is an ID, a date or a category, which transform comes next — answered with a probability for every option.")
                .size(11.5)
                .color(t.muted),
        );
        ui.add_space(6.0);
        for b in [DecisionBackend::Local, DecisionBackend::OpenJev, DecisionBackend::Llm] {
            if ui.radio(app.ai.settings.decision == b, b.label()).clicked() {
                app.ai.settings.decision = b;
                app.ai.hub.dirty = true;
            }
        }
        if app.ai.settings.decision == DecisionBackend::OpenJev {
            ui.add_space(6.0);
            egui::Grid::new("jev-grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label("Helper URL");
                if ui.add(egui::TextEdit::singleline(&mut app.ai.settings.openjev.base_url).desired_width(260.0)).changed() {
                    app.ai.hub.dirty = true;
                }
                ui.end_row();
                ui.label("SHIM_TOKEN");
                if ui.add(egui::TextEdit::singleline(&mut app.ai.settings.openjev.token).password(true).desired_width(260.0).hint_text("only when the helper is exposed")).changed() {
                    app.ai.hub.dirty = true;
                }
                ui.end_row();
            });
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(app.ai.hub.decision.is_none(), egui::Button::new("Try a decision")).on_hover_text("Asks: which chart for Order Date + Revenue? Is CustomerID an identifier?").clicked() {
                start_decision(app);
            }
            if app.ai.hub.decision.is_some() {
                ui.add(egui::Spinner::new().size(14.0));
            }
        });
        if let Some((d, err)) = &app.ai.hub.decision_result {
            if let Some(e) = err {
                ui.label(RichText::new(format!("Backend unavailable ({e}); answered by the personal brain instead.")).size(11.0).color(t.danger));
            }
            ui.label(RichText::new(format!("{} · {:.0} ms", d.backend, d.latency_ms)).size(11.0).color(t.muted));
            for a in &d.answers {
                ui.label(RichText::new(&a.key).strong().size(12.0));
                let mut probs = a.probs.clone();
                probs.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
                for (o, p) in probs.iter().take(5) {
                    prob_bar(ui, t, o, *p);
                }
            }
        }
    });
    card(ui, t, |ui| {
        h2(ui, "Set up OpenJEV (local decision model)");
        ui.label(
            RichText::new("OpenJEV is an open-weights 27B decision model: you describe a decision in plain words with your own labels, and it answers in one forward pass with calibrated probabilities (about 80 ms per short decision on one H100). VizCraft calls its helper's POST /v1/systemone API.")
                .size(11.5),
        );
        ui.add_space(6.0);
        ui.label(RichText::new("NVIDIA GPU (80 GB, FP8) — the measured recipe").strong().size(12.0));
        code(
            ui,
            t,
            "pip install \"vllm==0.29.0\" \"openai==3.16.2\" \"httpx==0.28.1\"\nhf download openjev/openjev --local-dir openjev\nvllm serve ./openjev --host 127.0.0.1 --served-model-name qwen --port 8000 \\\n  --enable-prefix-caching --max-model-len 16384 --max-logprobs 64 \\\n  --limit-mm-per-prompt '{\"image\":1}' --trust-remote-code --quantization fp8\nVLLM=http://localhost:8000/v1 TOKENIZER=./openjev READOUT_T=0.85 \\\nREADOUT_NOUL_T=1.829074 READOUT_NOUL_BIAS=0 READOUT_TARGETED=1 \\\nREADOUT_INSTR_STYLE=pyrepr SHIM_STAGGER=1 \\\npython openjev/helper/shim.py --host 127.0.0.1 --port 3000",
        );
        ui.add_space(4.0);
        ui.label(RichText::new("Apple silicon (MLX, text only)").strong().size(12.0));
        code(ui, t, "hf download openjev/openjev-MLX-4bit --local-dir openjev-mlx   # ~15 GB (8-bit: openjev/openjev-MLX, ~27 GB)\n# then run the repository's helper/shim_mlx.py as described in the model card");
        ui.add_space(4.0);
        ui.label(RichText::new("llama.cpp (GGUF, text only)").strong().size(12.0));
        code(ui, t, "hf download openjev/openjev-GGUF --include \"*Q4_K_M*\" --local-dir openjev-gguf   # 16.5 GB, fits a 24 GB GPU");
        ui.label(RichText::new("Smaller research variants (LoRA + decision head over Qwen): ZefanCai/Open-Jev-2B and ZefanCai/Open-Jev-9B. Method: zefan-cai.github.io/open-jev/story/#method").size(11.0).color(t.muted));
        ui.add_space(6.0);
        egui::Frame::new().fill(t.panel_alt).stroke(egui::Stroke::new(1.0, Color32::from_rgb(0xD9, 0x77, 0x06))).corner_radius(6.0).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Licence").strong().size(12.0));
            ui.label(
                RichText::new("OpenJEV weights are CC BY-NC 4.0: free for research and other non-commercial use, with attribution. Commercial use needs a licence from the authors (support@loopai.com). The helper and serving files are Apache 2.0. VizCraft only talks to a server you run; it doesn't ship the weights.")
                    .size(11.5),
            );
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Keep the helper on 127.0.0.1. If you expose it, set SHIM_TOKEN and enter it above.").size(11.0).color(t.muted));
    });
}

fn prob_bar(ui: &mut Ui, t: &Tokens, label: &str, p: f64) {
    ui.horizontal(|ui| {
        ui.add_sized([150.0, 16.0], egui::Label::new(RichText::new(vizcraft_ai::brain::kind_label(label)).size(11.0)).truncate());
        let (r, _) = ui.allocate_exact_size(egui::vec2(220.0, 10.0), egui::Sense::hover());
        ui.painter().rect_filled(r, 3.0, t.panel_alt);
        ui.painter().rect_filled(egui::Rect::from_min_size(r.min, egui::vec2(r.width() * p.clamp(0.0, 1.0) as f32, r.height())), 3.0, t.accent);
        ui.label(RichText::new(format!("{:.0}%", p * 100.0)).size(11.0).color(t.muted));
    });
}

/// The decider the settings select (OpenJEV / LLM), or None for the local brain.
pub fn remote_decider(app: &VizApp) -> Option<Box<dyn Decide>> {
    match app.ai.settings.decision {
        DecisionBackend::Local => None,
        DecisionBackend::OpenJev => Some(Box::new(OpenJevDecider { cfg: app.ai.settings.openjev.clone() })),
        DecisionBackend::Llm => app.ai.settings.active_provider().filter(|_| app.ai.settings.has_llm()).map(|p| Box::new(LlmDecider { provider: p.build() }) as Box<dyn Decide>),
    }
}

fn start_decision(app: &mut VizApp) {
    let primary = remote_decider(app);
    let local = app.ai.brain.decider();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let state = "Fields on the chart: Order Date (date), Revenue (measure). Table Customers has a column CustomerID with one distinct value per row.";
        let qs = vec![
            Question::choice("chart_type", "Which chart type shows these fields best?", &["line", "clusteredColumn", "area", "pie", "table", "card"]),
            Question::noul("customer_id_is_identifier", "Is CustomerID an identifier rather than a measure?"),
        ];
        let r = match primary {
            Some(p) => {
                let (d, e) = decide_with_fallback(p.as_ref(), &local, state, &qs);
                (d, e.map(|e| e.to_string()))
            }
            None => (local.decide(state, &qs).unwrap_or_default(), None),
        };
        let _ = tx.send(r);
    });
    app.ai.hub.decision = Some(rx);
    app.ai.hub.decision_result = None;
}

// ============================================================== brain

fn brain_tab(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let m = app.ai.brain.model.clone();
    card(ui, t, |ui| {
        ui.horizontal(|ui| {
            h2(ui, "Your personal brain");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut on = app.ai.settings.privacy.learn;
                let label = if on { "Learning on" } else { "Learning off" };
                if ui.toggle_value(&mut on, label).changed() {
                    app.ai.settings.privacy.learn = on;
                    app.ai.brain.enabled = on;
                    app.ai.hub.dirty = true;
                }
            });
        });
        ui.label(
            RichText::new("Like an ad account that learns from every campaign, this copy of VizCraft learns from every report you build: chart choices per kind of field, formatting habits, transform sequences, DAX patterns, questions and which suggestions you accept. Models train online, in Rust, on this device.")
                .size(11.5)
                .color(t.muted),
        );
        ui.add_space(8.0);
        let maturity = app.ai.brain.maturity();
        ui.horizontal(|ui| {
            stat(ui, t, &m.events.to_string(), "actions learned");
            stat(ui, t, &m.charts.counts.values().map(|c| c.values().sum::<f64>()).sum::<f64>().round().to_string(), "chart choices");
            stat(ui, t, &m.memory.items.len().to_string(), "questions remembered");
            stat(ui, t, &format!("{:.0}%", maturity * 100.0), "maturity");
        });
        let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(600.0), 6.0), egui::Sense::hover());
        ui.painter().rect_filled(r, 3.0, t.panel_alt);
        ui.painter().rect_filled(egui::Rect::from_min_size(r.min, egui::vec2(r.width() * maturity as f32, 6.0)), 3.0, t.accent);
    });
    ui.columns(2, |cols| {
        let [a, b] = cols else { return };
        card(a, t, |ui| {
            h2(ui, "Chart habits");
            let mut rows: Vec<(String, String, f64, f64)> = m
                .charts
                .counts
                .iter()
                .filter_map(|(ctx, c)| {
                    let n: f64 = c.values().sum();
                    let (k, v) = c.iter().max_by(|x, y| x.1.partial_cmp(y.1).unwrap_or(std::cmp::Ordering::Equal))?;
                    Some((ctx.replace('+', " + "), k.clone(), v / n, n))
                })
                .collect();
            rows.sort_by(|x, y| y.3.partial_cmp(&x.3).unwrap_or(std::cmp::Ordering::Equal));
            if rows.is_empty() {
                ui.label(RichText::new("Build a few visuals and your habits show up here.").color(t.muted).size(11.5));
            }
            for (ctx, k, share, n) in rows.iter().take(8) {
                ui.horizontal(|ui| {
                    ui.add_sized([150.0, 16.0], egui::Label::new(RichText::new(ctx).size(11.5)).truncate());
                    ui.label(RichText::new("→").color(t.muted));
                    ui.label(RichText::new(vizcraft_ai::brain::kind_label(k)).size(11.5).strong());
                    ui.label(RichText::new(format!("{:.0}% of {n:.0}", share * 100.0)).size(11.0).color(t.muted));
                });
            }
        });
        card(a, t, |ui| {
            h2(ui, "Formatting habits");
            let habits = app.ai.brain.model.prefs.habits("*", 0.6, 2.0);
            let themes = app.ai.brain.model.prefs.habits("report", 0.5, 1.0);
            if habits.is_empty() && themes.is_empty() {
                ui.label(RichText::new("None yet.").color(t.muted).size(11.5));
            }
            for h in habits.iter().chain(themes.iter()) {
                ui.label(RichText::new(format!("{} = {}   ({:.0}% of {:.0})", h.key, h.value, h.share * 100.0, h.n)).size(11.5));
            }
        });
        card(b, t, |ui| {
            h2(ui, "When you work");
            let max = m.hours.iter().copied().max().unwrap_or(0).max(1) as f32;
            let w = ui.available_width().min(420.0);
            let (r, _) = ui.allocate_exact_size(egui::vec2(w, 60.0), egui::Sense::hover());
            let bw = w / 24.0;
            for (h, c) in m.hours.iter().enumerate() {
                let hh = 50.0 * (*c as f32 / max);
                let x = r.left() + h as f32 * bw;
                ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(x + 1.0, r.top() + 50.0 - hh), egui::pos2(x + bw - 1.0, r.top() + 50.0)), 1.0, t.accent.gamma_multiply(0.8));
                if h % 6 == 0 {
                    ui.painter().text(egui::pos2(x, r.top() + 52.0), egui::Align2::LEFT_TOP, format!("{h}h"), egui::FontId::proportional(9.0), t.muted);
                }
            }
        });
        card(b, t, |ui| {
            h2(ui, "Patterns");
            let top: Vec<String> = m.measures.top().into_iter().take(4).map(|(p, n)| format!("{p} ({n:.0})")).collect();
            ui.label(RichText::new(format!("DAX: {}", if top.is_empty() { "—".into() } else { top.join(", ") })).size(11.5));
            let mut src: Vec<(String, f64)> = m.sequences.sources.iter().map(|(k, v)| (k.clone(), *v)).collect();
            src.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
            ui.label(RichText::new(format!("Sources: {}", if src.is_empty() { "—".into() } else { src.iter().take(4).map(|(k, v)| format!("{k} ({v:.0})")).collect::<Vec<_>>().join(", ") })).size(11.5));
            let arms: Vec<String> = m.bandit.arms.keys().map(|k| format!("{k} {:.0}%", m.bandit.mean(k) * 100.0)).collect();
            ui.label(RichText::new(format!("Suggestions you accept: {}", if arms.is_empty() { "—".into() } else { arms.join(", ") })).size(11.5));
        });
    });
    card(ui, t, |ui| {
        h2(ui, "Your data, your model");
        if let Some(p) = app.ai.brain.events_path() {
            ui.label(RichText::new(format!("Stored in {}", p.parent().map(|d| d.display().to_string()).unwrap_or_default())).size(11.0).color(t.muted));
        }
        ui.horizontal(|ui| {
            if ui.button("Export activity log…").clicked()
                && let Some(path) = rfd::FileDialog::new().set_file_name("vizcraft-brain-events.jsonl").save_file()
            {
                match app.ai.brain.export_events(&path) {
                    Ok(n) => app.toast(format!("Exported {n} events")),
                    Err(e) => app.error(format!("Couldn't export: {e}")),
                }
            }
            if ui.button("Fine-tune export…").on_hover_text("Writes sft.jsonl, decisions.jsonl and finetune_lora.py (HF transformers + peft) to LoRA-tune a small local model on your habits").clicked()
                && let Some(dir) = rfd::FileDialog::new().pick_folder()
            {
                match vizcraft_ai::finetune::export(&app.ai.brain.events(), &dir) {
                    Ok((a, b)) => app.toast(format!("Wrote {a} instruction and {b} decision examples")),
                    Err(e) => app.error(format!("Couldn't export: {e}")),
                }
            }
            let forget = ui.add(egui::Button::new(RichText::new("Forget everything").color(t.danger)));
            if forget.clicked() {
                app.ai.hub.confirm_forget = true;
            }
        });
        if app.ai.hub.confirm_forget {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Delete the activity log and everything learned? This can't be undone.").color(t.danger).size(11.5));
                if ui.button("Delete").clicked() {
                    app.ai.brain.forget();
                    app.ai.hub.confirm_forget = false;
                    app.toast("The personal brain was reset");
                }
                if ui.button("Cancel").clicked() {
                    app.ai.hub.confirm_forget = false;
                }
            });
        }
        ui.add_space(4.0);
        ui.label(
            RichText::new("Optional MLOps path: the fine-tune export turns your log into instruction data; finetune_lora.py LoRA-tunes e.g. Qwen2.5-0.5B/1.5B-Instruct with Hugging Face transformers + peft; serve the result with Ollama or llama.cpp and select it under Models & keys.")
                .size(11.0)
                .color(t.muted),
        );
    });
}

fn stat(ui: &mut Ui, t: &Tokens, value: &str, label: &str) {
    egui::Frame::new().fill(t.panel_alt).corner_radius(6.0).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(value).strong().size(18.0).color(t.accent));
            ui.label(RichText::new(label).size(10.5).color(t.muted));
        });
    });
}

// ============================================================== privacy

fn privacy_tab(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    card(ui, t, |ui| {
        h2(ui, "What leaves this machine");
        let p = &mut app.ai.settings.privacy;
        let mut changed = false;
        changed |= ui.checkbox(&mut p.send_sample_rows, "Send row values to cloud models (off: only schema and aggregates)").changed();
        ui.horizontal(|ui| {
            ui.label("Max rows a tool returns to the model");
            changed |= ui.add(egui::DragValue::new(&mut p.max_rows_to_model).range(1..=1000)).changed();
        });
        changed |= ui.checkbox(&mut p.auto_apply, "Apply copilot changes without asking").changed();
        if ui.checkbox(&mut p.learn, "Learn from my actions (personal brain, on device)").changed() {
            changed = true;
            app.ai.brain.enabled = p.learn;
        }
        ui.horizontal(|ui| {
            ui.label("Max agent steps per question");
            changed |= ui.add(egui::DragValue::new(&mut app.ai.settings.max_steps).range(2..=30)).changed();
        });
        if changed {
            app.ai.hub.dirty = true;
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new("Local models (Ollama, LM Studio, llama.cpp, vLLM, OpenJEV) keep everything on your machine. With a cloud provider, your question, the model's schema and the tool results it asks for are sent to that provider under its terms. The personal brain never leaves the device.")
                .size(11.5)
                .color(t.muted),
        );
    });
}
