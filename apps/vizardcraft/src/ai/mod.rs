//! AI in VizardCraft: the Copilot pane, the AI hub (providers, local models, OpenJEV, privacy, the
//! personal brain), the Formulator view (Data Formulator-style data threads) and the brain's
//! suggestion chips. The engine lives in the `vizardcraft-ai` crate; this module is the UI and the
//! glue to the app (tool execution on the UI thread, undo, tracking).
//!
//! Tracking hooks other modules call (all cheap, no-ops when learning is off):
//! - [`track_insert`] (report::insert_visual), [`track_field_added`] (report::panes::add_field),
//!   [`track_kind_changed`] (Visualizations gallery), [`track_format`] (format pane),
//!   [`track_filter`] (filters pane), [`track_theme`] (Themes menu),
//! - [`track_measure`] (model: new measure), [`track_transform`] / [`track_source`] (Power Query:
//!   a step added / a source connected) — for the modeling and Power Query modules to call.

pub mod chart;
pub mod copilot;
pub mod formulator;
pub mod hub;

use std::collections::{HashMap, HashSet};

use egui::{RichText, Ui};
use vizardcraft_ai::brain::{self, Brain, EventKind, FieldSig, SuggestContext, Suggestion, SuggestionAction};
use vizardcraft_ai::settings::AiSettings;
use vizardcraft_core::doc::*;

use crate::app::VizApp;
use crate::theme::Tokens;

pub struct AiState {
    pub settings: AiSettings,
    pub brain: Brain,
    pub copilot_open: bool,
    pub copilot: copilot::Copilot,
    pub hub: hub::HubUi,
    pub formulator: formulator::FormulatorUi,
    /// Visuals inserted this session whose first field set will be recorded as a chart choice.
    fresh: HashSet<u64>,
    /// Visual id → field-type key already recorded (to record each choice once).
    recorded: HashMap<u64, String>,
    /// Suggestion chips per selected visual (cached so Thompson sampling doesn't flicker).
    chips: Option<(String, Vec<Suggestion>)>,
    pub dismissed: HashSet<String>,
    pub show_traces: bool,
    pub env_hook_done: bool,
}

impl Default for AiState {
    fn default() -> Self {
        let settings = AiSettings::load();
        let brain = Brain::open(Brain::default_dir(), settings.privacy.learn);
        AiState {
            settings,
            brain,
            copilot_open: false,
            copilot: Default::default(),
            hub: Default::default(),
            formulator: Default::default(),
            fresh: HashSet::new(),
            recorded: HashMap::new(),
            chips: None,
            dismissed: HashSet::new(),
            show_traces: false,
            env_hook_done: false,
        }
    }
}

impl AiState {
    /// "Anthropic (Claude) · claude-sonnet-5-5", or the offline mode.
    pub fn mode_label(&self) -> String {
        match self.settings.active_provider() {
            Some(p) if self.settings.has_llm() => format!("{} · {}", p.label, p.model),
            Some(p) => format!("{} (no key) — offline mode", p.label),
            None => "Offline: rules + personal brain".into(),
        }
    }
}

// ============================================================== tracking

/// Records an action in the personal brain.
pub fn track(app: &mut VizApp, kind: EventKind) {
    app.ai.brain.record(kind);
    app.ai.chips = None;
}

pub fn sigs_of(app: &VizApp, v: &VisualSpec) -> Vec<FieldSig> {
    v.all_fields().map(|fu| brain::features::classify_use(&app.project, &app.built, fu)).collect()
}

fn kind_name(k: VisualKind) -> String {
    serde_json::to_value(k).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// A visual was inserted (its type is the user's choice once fields arrive).
pub fn track_insert(app: &mut VizApp, kind: VisualKind) {
    if let Some(id) = app.selected.first().copied() {
        app.ai.fresh.insert(id);
    }
    let _ = kind;
}

/// A field was added to a visual's well.
pub fn track_field_added(app: &mut VizApp, id: u64, role: &str, f: &FieldRef) {
    let Some(v) = app.project.element(id).and_then(|(_, e)| e.visual().cloned()) else { return };
    let field = brain::features::classify(&app.project, &app.built, f);
    track(app, EventKind::FieldAdded { visual: kind_name(v.kind), role: role.to_string(), field });
    record_choice(app, id, &v, "gallery");
}

/// Records "this user picked kind K for these field types" once per visual and field set.
fn record_choice(app: &mut VizApp, id: u64, v: &VisualSpec, source: &str) {
    let sigs = sigs_of(app, v);
    let has_val = sigs.iter().any(|s| s.is_value());
    let has_cat = sigs.iter().any(|s| !s.is_value());
    let complete = match v.kind {
        VisualKind::Card | VisualKind::MultiRowCard | VisualKind::Gauge | VisualKind::Kpi => has_val,
        VisualKind::Table | VisualKind::Slicer => sigs.len() >= 2 || matches!(v.kind, VisualKind::Slicer),
        _ => has_val && has_cat,
    };
    if !complete {
        return;
    }
    let key = format!("{}|{}", kind_name(v.kind), brain::features::context_key(&sigs));
    if app.ai.recorded.get(&id) == Some(&key) {
        return;
    }
    app.ai.recorded.insert(id, key);
    app.ai.fresh.remove(&id);
    track(app, EventKind::VisualCreated { kind: kind_name(v.kind), fields: sigs, source: source.to_string() });
}

/// The visual type of an existing visual was changed in the gallery.
pub fn track_kind_changed(app: &mut VizApp, id: u64, from: VisualKind, to: VisualKind) {
    let Some(v) = app.project.element(id).and_then(|(_, e)| e.visual().cloned()) else { return };
    let fields = sigs_of(app, &v);
    if fields.is_empty() || from == to {
        return;
    }
    app.ai.recorded.insert(id, format!("{}|{}", kind_name(to), brain::features::context_key(&fields)));
    track(app, EventKind::VisualKindChanged { from: kind_name(from), to: kind_name(to), fields });
}

fn prop_text(p: &Prop) -> Option<String> {
    Some(match p {
        Prop::Bool(b) => b.to_string(),
        Prop::Num(n) => format!("{n}"),
        Prop::Text(s) | Prop::Choice(s) => s.clone(),
        Prop::Color(c) => c.hex(),
        Prop::Fx(_) => return None,
    })
}

/// A formatting property was set by hand.
pub fn track_format(app: &mut VizApp, id: u64, key: &str, value: &Prop) {
    let Some(v) = prop_text(value) else { return };
    // Free text (titles) carries content, not style: learn only that a custom title was set.
    let v = if key.ends_with(".text") || key.ends_with("Text") { if v.is_empty() { "empty".into() } else { "custom".into() } } else { v };
    let scope = app.project.element(id).map(|(_, e)| vizardcraft_core::format::schema_key(e)).unwrap_or_else(|| "element".into());
    track(app, EventKind::Format { scope, key: key.to_string(), value: v });
}

pub fn track_theme(app: &mut VizApp, name: &str) {
    track(app, EventKind::Theme { name: name.to_string() });
}

pub fn track_filter(app: &mut VizApp, f: &FieldRef, kind: &str) {
    let field = brain::features::classify(&app.project, &app.built, f);
    track(app, EventKind::Filter { field, kind: kind.to_string() });
}

/// For the modeling module: a measure was created or edited.
pub fn track_measure(app: &mut VizApp, name: &str, dax: &str, format: &str) {
    track(app, EventKind::Measure { name: name.to_string(), dax: dax.to_string(), format: format.to_string() });
}

/// For the Power Query module: a step of type `step` (e.g. "RemoveColumns") was added to a
/// query whose source kind is `source` ("csv", "excel", "sql"…) after `history` step types.
pub fn track_transform(app: &mut VizApp, source: &str, history: &[String], step: &str) {
    track(app, EventKind::Transform { source: source.to_string(), history: history.to_vec(), step: step.to_string() });
}

/// For the Power Query module: a data source was connected.
pub fn track_source(app: &mut VizApp, kind: &str) {
    track(app, EventKind::Source { kind: kind.to_string() });
}

// ============================================================== frame hooks

/// Called every frame (from the app): background jobs, tool calls, windows, demo hooks.
pub fn dialogs(app: &mut VizApp, ctx: &egui::Context) {
    env_hooks(app);
    copilot::poll(app, ctx);
    hub::poll(app, ctx);
    formulator::poll(app, ctx);
    copilot::trace_window(app, ctx);
    hub::disclosure(app, ctx);
}

/// `VIZARDCRAFT_AI="copilot;ask=show sales by region;traces;formulate=profit ratio by region"` opens
/// AI UI for demos and screenshots.
fn env_hooks(app: &mut VizApp) {
    if app.ai.env_hook_done {
        return;
    }
    app.ai.env_hook_done = true;
    let Ok(spec) = std::env::var("VIZARDCRAFT_AI") else { return };
    for part in spec.split(';') {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        match k.trim() {
            "copilot" => app.ai.copilot_open = true,
            "ask" => {
                app.ai.copilot_open = true;
                copilot::ask(app, v.to_string());
            }
            "traces" => app.ai.show_traces = true,
            "formulate" => formulator::demo(app, v),
            "nodisclosure" => app.ai.settings.privacy.disclosure_seen = true,
            "seed" => seed_brain(app),
            _ => {}
        }
    }
}

/// Demo: teaches the brain a few habits (only into an in-memory brain, never the user's log).
fn seed_brain(app: &mut VizApp) {
    app.ai.brain = Brain::in_memory();
    let dm = vec![FieldSig::new("date", "Date"), FieldSig::new("measure", "Total Sales")];
    let cm = vec![FieldSig { ty: "category".into(), name: "Product".into(), card: "mid".into() }, FieldSig::new("measure", "Total Sales")];
    for i in 0..14 {
        app.ai.brain.record(EventKind::VisualCreated { kind: if i % 7 == 0 { "line" } else { "area" }.into(), fields: dm.clone(), source: "gallery".into() });
        app.ai.brain.record(EventKind::VisualCreated { kind: "clusteredBar".into(), fields: cm.clone(), source: "gallery".into() });
        app.ai.brain.record(EventKind::Format { scope: "area".into(), key: "labels.show".into(), value: "true".into() });
    }
    for _ in 0..5 {
        app.ai.brain.record(EventKind::Theme { name: "Ocean".into() });
        app.ai.brain.record(EventKind::Measure { name: "m".into(), dax: "DIVIDE([Profit], [Total Sales])".into(), format: "0.0%".into() });
        app.ai.brain.record(EventKind::Source { kind: "csv".into() });
    }
}

// ============================================================== suggestion chips

/// Brain suggestions for the selected visual, shown in the Visualizations pane's Build tab.
pub fn build_tab_chips(app: &mut VizApp, ui: &mut Ui, t: &Tokens, id: u64) {
    let Some(v) = app.project.element(id).and_then(|(_, e)| e.visual().cloned()) else { return };
    let props = app.project.element(id).map(|(_, e)| e.props.clone()).unwrap_or_default();
    let key = format!("{id}|{}|{}|{}", serde_json::to_string(&v.wells).unwrap_or_default(), kind_name(v.kind), app.ai.brain.model.events);
    if app.ai.chips.as_ref().map(|c| &c.0) != Some(&key) {
        let fields = sigs_of(app, &v);
        let measure = v.all_fields().find_map(|f| match &f.field {
            FieldRef::Measure { name, .. } => Some(name.clone()),
            _ => None,
        });
        let date_column = app.project.model.tables.iter().find_map(|tb| {
            vizardcraft_core::build::visible_columns(&app.project, &app.built, &tb.name).into_iter().find(|(_, ty)| ty.is_temporal()).map(|(c, _)| vizardcraft_core::state::col_ref(&tb.name, &c))
        });
        let current_format = props.iter().filter_map(|(k, p)| prop_text(p).map(|v| (k.clone(), v))).collect();
        let ctx = SuggestContext { fields, visual_kind: Some(kind_name(v.kind)), measure, date_column, current_format, max: 3, ..Default::default() };
        let mut s = app.ai.brain.suggest(&ctx);
        s.retain(|x| !app.ai.dismissed.contains(&format!("{id}|{}", x.id)));
        app.ai.chips = Some((key, s));
    }
    let chips = app.ai.chips.as_ref().map(|c| c.1.clone()).unwrap_or_default();
    if chips.is_empty() {
        return;
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
        crate::icons::paint(ui.painter(), r, "ai", t.accent);
        ui.label(RichText::new("For you").strong().size(11.5));
        ui.label(RichText::new("· learned on this device").size(10.5).color(t.muted)).on_hover_text("Suggestions from your personal brain: it learns from how you build visuals. Manage it in the AI hub.");
    });
    for s in chips {
        let mut accept = false;
        let mut dismiss = false;
        egui::Frame::new().fill(t.accent_soft).corner_radius(6.0).inner_margin(egui::Margin::symmetric(8, 5)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let prefix = match s.arm.as_str() {
                    "chart" => "Recommended:",
                    "format" => "Your style:",
                    "measure" => "Try:",
                    _ => "",
                };
                let r = ui.add(egui::Label::new(RichText::new(format!("{prefix} {}", s.title)).size(11.5).color(t.accent).strong()).sense(egui::Sense::click()));
                if r.on_hover_text(format!("{}\nClick to apply.", s.why)).clicked() {
                    accept = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new(RichText::new("✕").size(10.0).color(t.muted)).frame(false)).on_hover_text("Not useful").clicked() {
                        dismiss = true;
                    }
                });
            });
            ui.label(RichText::new(&s.why).size(10.5).color(t.muted));
        });
        ui.add_space(3.0);
        if accept {
            apply_suggestion(app, id, &s);
        }
        if dismiss {
            app.ai.dismissed.insert(format!("{id}|{}", s.id));
            track(app, EventKind::Suggestion { arm: s.arm.clone(), id: s.id.clone(), accepted: false });
        }
    }
}

/// Applies a brain suggestion to a visual.
pub fn apply_suggestion(app: &mut VizApp, id: u64, s: &Suggestion) {
    match &s.action {
        SuggestionAction::ChartType { kind } => {
            let Ok(k) = serde_json::from_value::<VisualKind>(serde_json::Value::String(kind.clone())) else { return };
            let from = app.project.element(id).and_then(|(_, e)| e.visual().map(|v| v.kind));
            app.checkpoint("Recommended visual type");
            if let Some(v) = app.project.element_mut(id).and_then(|e| e.visual_mut()) {
                crate::report::panes::remap_wells(v, k);
                v.kind = k;
            }
            if let Some(f) = from {
                track_kind_changed(app, id, f, k);
            }
        }
        SuggestionAction::Format { key, value } => {
            let p = vizardcraft_ai::tools::json_to_prop(&serde_json::Value::String(value.clone()));
            app.checkpoint("Apply your usual formatting");
            if let Some(e) = app.project.element_mut(id) {
                e.props.insert(key.clone(), p);
            }
        }
        SuggestionAction::Theme { name } => {
            if let Some(th) = vizardcraft_core::color::Theme::by_name(name) {
                app.checkpoint("Theme");
                app.project.report.theme = th;
            }
        }
        SuggestionAction::Measure { name, dax, format } => {
            let table = app.project.model.measures.first().map(|m| m.table.clone()).or_else(|| app.project.model.tables.first().map(|t| t.name.clone())).unwrap_or_default();
            app.checkpoint("Add suggested measure");
            app.project.model.measures.retain(|m| m.name != *name);
            app.project.model.measures.push(Measure { table, name: name.clone(), expression: dax.clone(), format: format.clone(), origin: "Personal brain".into(), ..Measure::default() });
            app.mark_model_dirty();
            app.toast(format!("Added measure [{name}]"));
        }
        SuggestionAction::Transform { .. } | SuggestionAction::Recall { .. } => {}
    }
    app.report.render_cache.clear();
    track(app, EventKind::Suggestion { arm: s.arm.clone(), id: s.id.clone(), accepted: true });
}

/// After the copilot or Formulator changed the project: refresh render caches.
pub fn after_ai_change(app: &mut VizApp) {
    app.generation += 1;
    app.cache.clear();
    app.report.on_rebuild();
}

/// A muted pill (for "AI" / "Offline" mode labels).
pub fn pill(ui: &mut Ui, t: &Tokens, text: &str, on: bool) -> egui::Response {
    let (fill, fg) = if on { (t.accent_soft, t.accent) } else { (t.panel_alt, t.muted) };
    egui::Frame::new()
        .fill(fill)
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(10.5).color(fg));
        })
        .response
}
