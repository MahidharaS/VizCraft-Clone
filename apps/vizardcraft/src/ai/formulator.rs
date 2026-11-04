//! The Formulator view — VizardCraft's take on Microsoft Data Formulator (MIT): build charts from a
//! *concept encoding* (x, y, color, size, facets) plus natural language. Type a field that
//! doesn't exist ("Profit ratio") and it is derived (DAX written by the AI, or by rules), then
//! validated by running it. Every step is a node in a branching *data thread*; follow-ups
//! branch from any node; charts go to the report one by one or composed into a page.
//!
//! Without an AI provider it still works: rule-based Q&A, rule-based derivations and the
//! personal brain's chart choice, clearly labelled.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use egui::{Color32, RichText, Ui};
use vizardcraft_ai::brain::{EventKind, SuggestionAction};
use vizardcraft_ai::decide::{Decision, Question};
use vizardcraft_ai::formulate::{self, DerivedField, Encoding};
use vizardcraft_ai::provider::{ChatRequest, Message};
use vizardcraft_core::build::{self, Built};
use vizardcraft_core::doc::*;
use vizardcraft_core::vquery::VisualData;

use crate::app::VizApp;
use crate::theme::Tokens;

#[derive(Clone)]
pub struct Node {
    pub id: u64,
    pub parent: Option<u64>,
    pub prompt: String,
    pub enc: Encoding,
    pub derived: Vec<DerivedField>,
    pub chart: VisualKind,
    /// The chart type is chosen automatically (brain / decision model) until the user picks one.
    pub auto_chart: bool,
    pub explanation: String,
    /// "AI", "Rules", "Manual".
    pub source: String,
    pub why: String,
    pub spec: VisualSpec,
    pub data: Option<Rc<VisualData>>,
    pub error: Option<String>,
    pub picked: bool,
    pub pending: bool,
    pub decision: Option<Vec<(String, f64)>>,
    scratch: Option<Rc<(Project, Built)>>,
}

#[derive(Default)]
pub struct FormulatorUi {
    pub nodes: Vec<Node>,
    pub selected: Option<u64>,
    pub prompt: String,
    pub draft: Encoding,
    pub shelf_input: HashMap<String, String>,
    pub preview: Option<String>,
    pub tab: usize,
    next: u64,
    job: Option<(u64, Receiver<Result<serde_json::Value, String>>)>,
    decision_job: Option<(u64, Receiver<(Decision, Option<String>)>)>,
    /// The draft was loaded from this node (reloaded when the selection changes).
    loaded: Option<u64>,
}

#[derive(Clone, Debug)]
struct DragName(String);

fn node<'a>(app: &'a VizApp, id: u64) -> Option<&'a Node> {
    app.ai.formulator.nodes.iter().find(|n| n.id == id)
}
fn node_mut(app: &mut VizApp, id: u64) -> Option<&mut Node> {
    app.ai.formulator.nodes.iter_mut().find(|n| n.id == id)
}

fn kind_name(k: VisualKind) -> String {
    serde_json::to_value(k).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}
fn kind_from(s: &str) -> Option<VisualKind> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
}

// ============================================================== formulating

/// Starts a new step from `parent` with a prompt and an encoding.
pub fn formulate(app: &mut VizApp, parent: Option<u64>, prompt: String, enc: Encoding) {
    let id = {
        let f = &mut app.ai.formulator;
        f.next += 1;
        f.next
    };
    let chart = kind_from(&enc.chart);
    let n = Node {
        id,
        parent,
        prompt: prompt.clone(),
        enc: enc.clone(),
        derived: parent.and_then(|p| node(app, p)).map(|p| p.derived.clone()).unwrap_or_default(),
        chart: chart.unwrap_or(VisualKind::ClusteredColumn),
        auto_chart: chart.is_none(),
        explanation: String::new(),
        source: "Rules".into(),
        why: String::new(),
        spec: VisualSpec::default(),
        data: None,
        error: None,
        picked: false,
        pending: true,
        decision: None,
        scratch: None,
    };
    app.ai.formulator.nodes.push(n);
    app.ai.formulator.selected = Some(id);
    app.ai.formulator.loaded = None;
    let unknown = formulate::unknown_fields(&app.project, &app.built, &enc, &[]);
    let use_ai = app.ai.settings.has_llm() && (!prompt.trim().is_empty() || !unknown.is_empty());
    if use_ai && let Some(p) = app.ai.settings.active_provider() {
        let parent_desc = parent.and_then(|p| node(app, p)).map(|n| format!("prompt \"{}\", encoding {}, derived {}", n.prompt, serde_json::to_string(&n.enc).unwrap_or_default(), n.derived.iter().map(|d| format!("{} = {}", d.name, d.dax)).collect::<Vec<_>>().join("; ")));
        let text = formulate::ai_prompt(&app.project, &app.built, &prompt, &enc, parent_desc.as_deref(), &app.ai.brain.profile_summary());
        let provider = p.build();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let req = ChatRequest { messages: vec![Message::user(text)], tools: vec![], temperature: Some(0.1), max_tokens: 1500, json_mode: true };
            let r = provider.chat(&req).map_err(|e| e.to_string()).and_then(|r| vizardcraft_ai::extract_json(&r.content).ok_or_else(|| "The model didn't answer with JSON.".to_string()));
            let _ = tx.send(r);
        });
        if let Some(n) = node_mut(app, id) {
            n.source = "AI".into();
        }
        app.ai.formulator.job = Some((id, rx));
        return;
    }
    rules_path(app, id);
}

fn rules_path(app: &mut VizApp, id: u64) {
    let Some(mut n) = node(app, id).cloned() else { return };
    if n.enc.x.is_empty() && n.enc.y.is_empty() && !n.prompt.trim().is_empty() {
        let r = formulate::rules_encoding(&app.project, &app.built, &n.prompt);
        // Keep what the parent showed when the follow-up only refines it.
        let parent_enc = n.parent.and_then(|p| node(app, p)).map(|p| p.enc.clone()).unwrap_or_default();
        n.enc = Encoding {
            x: if r.x.is_empty() { parent_enc.x.clone() } else { r.x },
            y: if r.y.is_empty() { parent_enc.y.clone() } else { r.y },
            color: if r.color.is_empty() { n.enc.color.clone() } else { r.color },
            chart: if r.chart.is_empty() { n.enc.chart.clone() } else { r.chart },
            ..n.enc.clone()
        };
        if let Some(k) = kind_from(&n.enc.chart) {
            n.chart = k;
            n.auto_chart = false;
        }
    }
    for name in formulate::unknown_fields(&app.project, &app.built, &n.enc, &n.derived) {
        match formulate::rules_derive(&app.project, &app.built, name.trim_matches(['[', ']'])) {
            Some(d) => n.derived.push(d),
            None => n.error = Some(format!("Couldn't work out \"{name}\" without an AI model. Connect one in the AI hub, or use existing fields.")),
        }
    }
    n.source = if n.derived.is_empty() && n.prompt.is_empty() { "Manual".into() } else { "Rules".into() };
    if n.explanation.is_empty() {
        n.explanation = describe(&n.enc);
    }
    if let Some(slot) = node_mut(app, id) {
        *slot = n;
    }
    finish(app, id);
}

fn describe(enc: &Encoding) -> String {
    let y = enc.y.join(", ");
    let x = enc.x.join(", ");
    match (y.is_empty(), x.is_empty()) {
        (false, false) => format!("{y} by {x}"),
        (false, true) => y,
        (true, false) => x,
        _ => String::new(),
    }
}

/// Validates derived fields, picks the chart, builds the spec and computes the data.
fn finish(app: &mut VizApp, id: u64) {
    let Some(mut n) = node(app, id).cloned() else { return };
    n.pending = false;
    // Encoded names of derived fields → measure references.
    for c in Encoding::CHANNELS {
        for f in n.enc.channel_mut(c).iter_mut() {
            if n.derived.iter().any(|d| d.name.eq_ignore_ascii_case(f.trim_matches(['[', ']']))) && !f.starts_with('[') {
                *f = format!("[{f}]");
            }
        }
    }
    let (q, qb) = formulate::validate(&app.project, &app.built, &mut n.derived);
    let bad: Vec<String> = n.derived.iter().filter_map(|d| d.error.as_ref().map(|e| format!("{}: {e}", d.name))).collect();
    if !bad.is_empty() {
        n.error = Some(format!("Some derived fields didn't validate — {}", bad.join("; ")));
    }
    let sigs = formulate::signatures(&q, &qb, &n.enc);
    if n.auto_chart {
        if let Some(top) = app.ai.brain.recommend_chart(&sigs, 1).into_iter().next()
            && let SuggestionAction::ChartType { kind } = &top.action
            && let Some(k) = kind_from(kind)
        {
            n.chart = k;
            n.why = top.why.clone();
        }
    }
    respec(&mut n, &q, &qb);
    n.scratch = Some(Rc::new((q, qb)));
    let start_decision = n.auto_chart && n.error.is_none() && !sigs.is_empty();
    if let Some(slot) = node_mut(app, id) {
        *slot = n;
    }
    app.ai.brain.record(EventKind::Question { text: node(app, id).map(|n| n.prompt.clone()).unwrap_or_default(), resolution: node(app, id).map(|n| format!("{} of {}", n.chart.label(), describe(&n.enc))).unwrap_or_default(), accepted: true });
    if start_decision {
        start_chart_decision(app, id, &sigs);
    }
}

fn respec(n: &mut Node, q: &Project, qb: &Built) {
    let (spec, missing) = formulate::encoding_to_spec(q, qb, &n.enc, n.chart);
    let data = formulate::compute(q, qb, &spec);
    if !missing.is_empty() && n.error.is_none() {
        n.error = Some(format!("Not found: {}", missing.join(", ")));
    }
    n.spec = spec;
    n.data = Some(Rc::new(data));
}

/// Asks the configured decision backend (OpenJEV / LLM) which chart fits, in the background.
fn start_chart_decision(app: &mut VizApp, id: u64, sigs: &[vizardcraft_ai::brain::FieldSig]) {
    let Some(primary) = super::hub::remote_decider(app) else { return };
    let local = app.ai.brain.decider();
    let options: Vec<String> = app.ai.brain.model.charts.rank(sigs).into_iter().take(6).map(|c| c.kind).collect();
    let prompt = node(app, id).map(|n| n.prompt.clone()).unwrap_or_default();
    let state = format!("A user wants a chart. Request: \"{prompt}\". Fields: {}.", sigs.iter().map(|s| format!("{} ({})", s.name, s.ty)).collect::<Vec<_>>().join(", "));
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let refs: Vec<&str> = options.iter().map(String::as_str).collect();
        let q = vec![Question::choice("chart_type", "Which chart type shows this best?", &refs)];
        let (d, e) = vizardcraft_ai::decide::decide_with_fallback(primary.as_ref(), &local, &state, &q);
        let _ = tx.send((d, e.map(|e| e.to_string())));
    });
    app.ai.formulator.decision_job = Some((id, rx));
}

pub fn poll(app: &mut VizApp, ctx: &egui::Context) {
    if let Some((id, rx)) = app.ai.formulator.job.take() {
        match rx.try_recv() {
            Ok(Ok(v)) => {
                let (enc, derived, why) = formulate::parse_ai(&v);
                if let Some(n) = node_mut(app, id) {
                    // The user's own encodings win; the AI fills the gaps.
                    for c in Encoding::CHANNELS {
                        if n.enc.channel(c).is_empty() {
                            *n.enc.channel_mut(c) = enc.channel(c).clone();
                        }
                    }
                    if n.auto_chart && let Some(k) = kind_from(&enc.chart) {
                        n.chart = k;
                        n.auto_chart = false;
                        n.why = "Chosen by the AI model.".into();
                    }
                    for d in derived {
                        n.derived.retain(|x| !x.name.eq_ignore_ascii_case(&d.name));
                        n.derived.push(d);
                    }
                    n.explanation = why;
                }
                // Anything still unknown: rules.
                rules_path(app, id);
            }
            Ok(Err(e)) => {
                if let Some(n) = node_mut(app, id) {
                    n.source = "Rules".into();
                    n.explanation = format!("AI unavailable ({e}); used rules.");
                }
                rules_path(app, id);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                app.ai.formulator.job = Some((id, rx));
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            Err(_) => rules_path(app, id),
        }
    }
    if let Some((id, rx)) = app.ai.formulator.decision_job.take() {
        match rx.try_recv() {
            Ok((d, err)) => {
                if let Some(a) = d.get("chart_type").cloned()
                    && let Some(n) = node_mut(app, id)
                    && n.auto_chart
                {
                    n.decision = Some(a.probs.clone());
                    if let Some(k) = kind_from(&a.choice) {
                        n.chart = k;
                        n.why = match &err {
                            None => format!("{} chose it ({:.0}% confident).", d.backend, a.confidence * 100.0),
                            Some(e) => format!("Decision model unavailable ({e}); your brain chose it."),
                        };
                        if let Some(s) = n.scratch.clone() {
                            respec(n, &s.0, &s.1);
                        }
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                app.ai.formulator.decision_job = Some((id, rx));
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            Err(_) => {}
        }
    }
}

/// Demo / screenshot hook: `formulate=prompt1|prompt2|<sibling prompt`.
pub fn demo(app: &mut VizApp, spec: &str) {
    app.view = crate::app::View::Formulator;
    if app.needs_refresh {
        app.needs_refresh = false;
        app.refresh();
    }
    let mut prev: Option<u64> = None;
    for p in spec.split('|').filter(|s| !s.trim().is_empty()) {
        let (parent, text) = match p.strip_prefix('<') {
            Some(rest) => (prev.and_then(|x| node(app, x)).and_then(|n| n.parent), rest),
            None => (prev, p),
        };
        let mut enc = Encoding::default();
        let mut prompt = text.trim().to_string();
        // "y=Profit ratio;x=Store[Region]" style encodings.
        if prompt.contains('=') && !prompt.contains(' ') || prompt.starts_with("enc:") {
            prompt = prompt.trim_start_matches("enc:").to_string();
            for kv in prompt.split(',') {
                if let Some((c, f)) = kv.split_once('=') {
                    if c == "chart" {
                        enc.chart = f.to_string();
                    } else if Encoding::CHANNELS.contains(&c) {
                        enc.channel_mut(c).push(f.replace('_', " "));
                    }
                }
            }
            prompt.clear();
        }
        formulate(app, parent, prompt, enc);
        prev = app.ai.formulator.selected;
    }
    if let Some(first) = app.ai.formulator.nodes.first().map(|n| n.id) {
        if let Some(n) = node_mut(app, first) {
            n.picked = true;
        }
    }
}

// ============================================================== report

fn element_for(n: &Node) -> Element {
    let title = if n.prompt.is_empty() { describe(&n.enc) } else { n.prompt.clone() };
    let mut el = Element { name: title.clone(), w: 560.0, h: 320.0, kind: ElementKind::Visual(n.spec.clone()), ..Element::default() };
    el.props.insert("title.text".into(), Prop::Text(cap_first(&title)));
    el
}

fn cap_first(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn merge_derived(app: &mut VizApp, nodes: &[Node]) {
    let all: Vec<DerivedField> = nodes.iter().flat_map(|n| n.derived.iter().filter(|d| d.error.is_none()).cloned()).collect();
    if !all.is_empty() {
        app.project = formulate::with_derived(&app.project, &all);
        app.mark_model_dirty();
    }
}

pub fn add_to_report(app: &mut VizApp, id: u64) {
    let Some(n) = node(app, id).cloned() else { return };
    if n.data.as_ref().is_some_and(|d| d.error.is_some()) || n.pending {
        app.error("This step has no chart yet.");
        return;
    }
    app.checkpoint("Add Formulator chart to report");
    merge_derived(app, std::slice::from_ref(&n));
    let mut el = element_for(&n);
    el.id = app.project.next_id();
    let page_id = app.page;
    if let Some(pg) = app.project.page_mut(page_id) {
        let (x, y) = vizardcraft_ai::tools::free_spot(pg, el.w, el.h);
        el.x = x;
        el.y = y;
        let eid = el.id;
        pg.elements.push(el);
        app.selected = vec![eid];
    }
    let sigs = crate::ai::sigs_of(app, &n.spec);
    crate::ai::track(app, EventKind::VisualCreated { kind: kind_name(n.chart), fields: sigs, source: "formulator".into() });
    crate::ai::after_ai_change(app);
    app.toast("Added to the report page");
}

/// Makes a report page from the picked steps (or the selected thread's path).
pub fn compose(app: &mut VizApp) {
    let mut picked: Vec<Node> = app.ai.formulator.nodes.iter().filter(|n| n.picked && !n.pending && n.data.as_ref().is_some_and(|d| d.error.is_none())).cloned().collect();
    if picked.is_empty() {
        let mut cur = app.ai.formulator.selected;
        while let Some(id) = cur {
            let Some(n) = node(app, id).cloned() else { break };
            cur = n.parent;
            if n.data.as_ref().is_some_and(|d| d.error.is_none()) {
                picked.insert(0, n);
            }
        }
    }
    if picked.is_empty() {
        app.error("Pick some steps first (the checkbox on each card).");
        return;
    }
    app.checkpoint("Compose report from Formulator");
    merge_derived(app, &picked);
    let pid = app.project.next_id();
    let mut page = Page::new(pid, &app.project.free_table_name("Formulator report"), PageKind::Report);
    page.name = format!("Formulator: {}", cap_first(picked.first().map(|n| if n.prompt.is_empty() { describe(&n.enc) } else { n.prompt.clone() }).unwrap_or_default().as_str()).chars().take(40).collect::<String>());
    let cols = if picked.len() == 1 { 1 } else { 2 };
    let (gap, w) = (20.0, if cols == 1 { 1240.0 } else { 610.0 });
    let h = if picked.len() <= 2 { 680.0 } else { 330.0 };
    for (i, n) in picked.iter().enumerate() {
        let mut el = element_for(n);
        el.id = app.project.next_id();
        el.x = gap + (i % cols) as f32 * (w + gap);
        el.y = gap + (i / cols) as f32 * (h + gap);
        el.w = w;
        el.h = h;
        page.elements.push(el);
    }
    page.height = page.height.max(gap + (picked.len().div_ceil(cols)) as f32 * (h + gap));
    app.project.report.pages.push(page);
    app.page = pid;
    app.view = crate::app::View::Report;
    crate::ai::track(app, EventKind::Page { kind: "formulator".into() });
    crate::ai::after_ai_change(app);
    app.toast(format!("Composed a page from {} charts", picked.len()));
}

// ============================================================== view

pub fn show(app: &mut VizApp, ui: &mut Ui) {
    let t = app.tokens();
    let pane = egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::same(10)).stroke(egui::Stroke::new(1.0, t.border));
    egui::Panel::left("fm-data").default_size(230.0).size_range(190.0..=360.0).frame(pane).show(ui, |ui| data_pane(app, ui, &t));
    egui::Panel::left("fm-threads").default_size(280.0).size_range(220.0..=420.0).frame(egui::Frame::new().fill(t.panel_alt).inner_margin(egui::Margin::same(10)).stroke(egui::Stroke::new(1.0, t.border))).show(ui, |ui| threads(app, ui, &t));
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg).inner_margin(egui::Margin::symmetric(18, 12))).show(ui, |ui| main(app, ui, &t));
}

fn field_chip(ui: &mut Ui, t: &Tokens, name: &str, label: &str, measure: bool) {
    let id = ui.scope_id().with(("fm-field", name));
    ui.dnd_drag_source(id, DragName(name.to_string()), |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(if measure { "∑" } else { "▤" }).size(11.0).color(if measure { t.good } else { t.muted }));
            ui.label(RichText::new(label).size(11.5));
        });
    })
    .response
    .on_hover_text("Drag onto an encoding channel");
}

fn data_pane(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    ui.label(RichText::new("Data").strong().size(13.0));
    ui.label(RichText::new("Drag fields to the encoding shelf, or type new ones to derive.").size(10.5).color(t.muted));
    ui.add_space(4.0);
    let tables: Vec<String> = app.project.model.tables.iter().filter(|x| !x.hidden).map(|x| x.name.clone()).collect();
    let preview_h = if app.ai.formulator.preview.is_some() { 230.0 } else { 0.0 };
    egui::ScrollArea::vertical().id_salt("fm-fields").max_height((ui.available_height() - preview_h).max(120.0)).auto_shrink([false, false]).show(ui, |ui| {
        for tname in &tables {
            let rows = app.built.table(tname).map(|d| d.len()).unwrap_or(0);
            let open = egui::CollapsingHeader::new(RichText::new(tname).strong().size(12.0)).id_salt(("fm-t", tname)).default_open(tables.len() <= 3).show(ui, |ui| {
                for m in app.project.model.measures.iter().filter(|m| &m.table == tname && !m.hidden) {
                    field_chip(ui, t, &format!("[{}]", m.name), &m.name, true);
                }
                for (c, ty) in build::visible_columns(&app.project, &app.built, tname) {
                    field_chip(ui, t, &format!("{tname}[{c}]"), &c, ty.is_numeric() && !c.to_lowercase().ends_with("id"));
                }
            });
            let hr = open.header_response.on_hover_text(format!("{rows} rows · click the eye to preview"));
            if hr.secondary_clicked() || hr.double_clicked() {
                app.ai.formulator.preview = Some(tname.clone());
            }
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                if ui.add(egui::Button::new(RichText::new(format!("Preview · {rows} rows")).size(10.5).color(t.accent)).frame(false)).clicked() {
                    app.ai.formulator.preview = if app.ai.formulator.preview.as_deref() == Some(tname) { None } else { Some(tname.clone()) };
                }
            });
        }
    });
    if let Some(name) = app.ai.formulator.preview.clone()
        && let Some(d) = app.built.table(&name).cloned()
    {
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{name} preview")).strong().size(11.5));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new("✕").frame(false)).clicked() {
                    app.ai.formulator.preview = None;
                }
            });
        });
        egui::ScrollArea::both().id_salt("fm-preview").auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("fm-preview-grid").striped(true).spacing([8.0, 2.0]).show(ui, |ui| {
                for c in d.columns.iter().take(8) {
                    ui.label(RichText::new(c).strong().size(10.5));
                }
                ui.end_row();
                for r in 0..d.len().min(40) {
                    for c in 0..d.columns.len().min(8) {
                        let v = d.cell(r, c);
                        let s = match (v, d.types.get(c)) {
                            (vizardcraft_core::scalar::Scalar::Num(n), Some(ty)) => vizardcraft_core::vquery::format_value(*n, ty.default_format(), *ty),
                            _ => v.text(),
                        };
                        ui.label(RichText::new(s).size(10.5));
                    }
                    ui.end_row();
                }
            });
        });
    }
}

fn threads(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Data threads").strong().size(13.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new(RichText::new("+ New").size(11.5))).on_hover_text("Start a new thread from the data").clicked() {
                app.ai.formulator.selected = None;
                app.ai.formulator.draft = Encoding::default();
                app.ai.formulator.loaded = None;
                app.ai.formulator.prompt.clear();
            }
        });
    });
    ui.label(RichText::new("Each step derives data and a chart; follow-ups branch from the selected step.").size(10.5).color(t.muted));
    ui.add_space(6.0);
    if app.ai.formulator.nodes.is_empty() {
        ui.add_space(20.0);
        ui.label(RichText::new("No threads yet.").color(t.muted));
        return;
    }
    let mut order: Vec<(u64, usize)> = Vec::new();
    fn walk(nodes: &[Node], parent: Option<u64>, depth: usize, out: &mut Vec<(u64, usize)>) {
        for n in nodes.iter().filter(|n| n.parent == parent) {
            out.push((n.id, depth));
            walk(nodes, Some(n.id), depth + 1, out);
        }
    }
    walk(&app.ai.formulator.nodes, None, 0, &mut order);
    let theme = app.project.report.theme.clone();
    let mut delete: Option<u64> = None;
    egui::ScrollArea::vertical().id_salt("fm-threads-scroll").auto_shrink([false, false]).show(ui, |ui| {
        for (id, depth) in order {
            let Some(n) = node(app, id).cloned() else { continue };
            let indent = (depth.min(6) as f32) * 14.0;
            let sel = app.ai.formulator.selected == Some(id);
            // Fixed card width so the card never pushes the resizable panel wider.
            let card_w = (ui.available_width() - indent - if depth > 0 { 8.0 } else { 0.0 } - 14.0).max(120.0);
            ui.horizontal(|ui| {
                ui.add_space(indent);
                if depth > 0 {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 20.0), egui::Sense::hover());
                    ui.painter().line_segment([r.left_top(), r.left_center()], egui::Stroke::new(1.5, t.border));
                    ui.painter().line_segment([r.left_center(), r.right_center()], egui::Stroke::new(1.5, t.border));
                }
                let resp = egui::Frame::new()
                    .fill(t.panel)
                    .stroke(egui::Stroke::new(if sel { 2.0 } else { 1.0 }, if sel { t.accent } else { t.border }))
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| ui.vertical(|ui| {
                        ui.set_width(card_w);
                        let w = card_w;
                        let (r, _) = ui.allocate_exact_size(egui::vec2(w, 84.0), egui::Sense::hover());
                        if n.pending {
                            ui.painter().rect_filled(r, 6.0, t.panel_alt);
                            ui.put(egui::Rect::from_center_size(r.center(), egui::vec2(18.0, 18.0)), egui::Spinner::new().color(t.accent));
                        } else if let Some(d) = &n.data {
                            super::chart::paint(ui, r, &theme, &n.spec, d, true);
                        }
                        let title = if n.prompt.is_empty() { describe(&n.enc) } else { n.prompt.clone() };
                        ui.label(RichText::new(cap_first(&title)).size(11.5).strong());
                        ui.horizontal_wrapped(|ui| {
                            let (fill, col) = match n.source.as_str() {
                                "AI" => (t.accent_soft, t.accent),
                                _ => (t.panel_alt, t.muted),
                            };
                            egui::Frame::new().fill(fill).corner_radius(8.0).inner_margin(egui::Margin::symmetric(6, 1)).show(ui, |ui| {
                                ui.label(RichText::new(&n.source).size(9.5).color(col));
                            });
                            ui.label(RichText::new(n.chart.label()).size(10.0).color(t.muted));
                            for d in n.derived.iter().take(2) {
                                ui.label(RichText::new(format!("ƒ {}", d.name)).size(10.0).color(if d.error.is_some() { t.danger } else { t.good }));
                            }
                        });
                    }))
                    .response;
                let resp = resp.interact(egui::Sense::click());
                if resp.clicked() {
                    app.ai.formulator.selected = Some(id);
                }
                resp.context_menu(|ui| {
                    if ui.button("Add to report").clicked() {
                        add_to_report(app, id);
                        ui.close();
                    }
                    if ui.button("Delete step and branches").clicked() {
                        delete = Some(id);
                        ui.close();
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.add_space(indent + if depth > 0 { 12.0 } else { 0.0 });
                let mut picked = n.picked;
                if ui.checkbox(&mut picked, RichText::new("Pick for report").size(10.5).color(t.muted)).changed()
                    && let Some(m) = node_mut(app, id)
                {
                    m.picked = picked;
                }
            });
            ui.add_space(4.0);
        }
    });
    if let Some(id) = delete {
        let mut gone = vec![id];
        let mut i = 0;
        while i < gone.len() {
            let cur = gone[i];
            gone.extend(app.ai.formulator.nodes.iter().filter(|n| n.parent == Some(cur)).map(|n| n.id));
            i += 1;
        }
        app.ai.formulator.nodes.retain(|n| !gone.contains(&n.id));
        if app.ai.formulator.selected.is_some_and(|s| gone.contains(&s)) {
            app.ai.formulator.selected = None;
        }
    }
}

fn main(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    // Toolbar.
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
        crate::icons::paint(ui.painter(), r, "formulator", t.accent);
        ui.label(RichText::new("Formulator").strong().size(17.0));
        let on = app.ai.settings.has_llm();
        crate::ai::pill(ui, t, &if on { format!("AI: {}", app.ai.mode_label()) } else { "Offline: rules + personal brain".into() }, on);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let n_picked = app.ai.formulator.nodes.iter().filter(|n| n.picked).count();
            if ui.add(egui::Button::new(RichText::new(format!("Compose report ({n_picked})")).color(Color32::WHITE)).fill(t.accent)).on_hover_text("Make a report page from the picked steps (or the selected thread)").clicked() {
                compose(app);
            }
            if let Some(id) = app.ai.formulator.selected
                && ui.button("Add to report").on_hover_text("Pin this chart to the current report page").clicked()
            {
                add_to_report(app, id);
            }
        });
    });
    ui.add_space(4.0);
    super::hub::banner(app, ui, t);
    // Load the selected node's encoding into the shelf.
    if app.ai.formulator.loaded != app.ai.formulator.selected {
        app.ai.formulator.loaded = app.ai.formulator.selected;
        app.ai.formulator.draft = app.ai.formulator.selected.and_then(|id| node(app, id)).map(|n| Encoding { chart: if n.auto_chart { String::new() } else { kind_name(n.chart) }, ..n.enc.clone() }).unwrap_or_default();
    }
    shelf(app, ui, t);
    ui.add_space(8.0);
    let Some(id) = app.ai.formulator.selected else {
        empty(app, ui, t);
        return;
    };
    let Some(n) = node(app, id).cloned() else { return };
    egui::Frame::new().fill(t.panel).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8.0).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(cap_first(if n.prompt.is_empty() { &n.explanation } else { &n.prompt })).strong().size(14.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                chart_picker(app, ui, id, &n);
            });
        });
        if !n.explanation.is_empty() && !n.prompt.is_empty() {
            ui.label(RichText::new(&n.explanation).size(11.5).color(t.muted));
        }
        if let Some(e) = &n.error {
            ui.label(RichText::new(e).size(11.5).color(t.danger));
        }
        let h = (ui.available_height() * 0.58).clamp(220.0, 520.0);
        let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::hover());
        if n.pending {
            ui.put(egui::Rect::from_center_size(r.center(), egui::vec2(28.0, 28.0)), egui::Spinner::new().size(28.0).color(t.accent));
        } else if let Some(d) = &n.data {
            super::chart::paint(ui, r, &app.project.report.theme, &n.spec, d, false);
        }
        recommendations(app, ui, t, id, &n);
    });
    ui.add_space(8.0);
    details(app, ui, t, &n);
}

fn chart_picker(app: &mut VizApp, ui: &mut Ui, id: u64, n: &Node) {
    const CHOICES: &[VisualKind] = &[
        VisualKind::ClusteredColumn,
        VisualKind::ClusteredBar,
        VisualKind::StackedColumn,
        VisualKind::Line,
        VisualKind::Area,
        VisualKind::Scatter,
        VisualKind::Pie,
        VisualKind::Donut,
        VisualKind::Treemap,
        VisualKind::Table,
        VisualKind::Matrix,
        VisualKind::HeatMap,
        VisualKind::Card,
        VisualKind::Map,
    ];
    let mut pick = None;
    egui::ComboBox::from_id_salt(("fm-chart", id)).selected_text(format!("{}{}", n.chart.label(), if n.auto_chart { " (auto)" } else { "" })).width(200.0).show_ui(ui, |ui| {
        for k in CHOICES {
            if ui.selectable_label(n.chart == *k, k.label()).clicked() {
                pick = Some(*k);
            }
        }
    });
    if let Some(k) = pick {
        set_chart(app, id, k, false);
    }
}

fn set_chart(app: &mut VizApp, id: u64, k: VisualKind, from_suggestion: bool) {
    let Some(n) = node_mut(app, id) else { return };
    let from = n.chart;
    n.chart = k;
    n.auto_chart = false;
    n.error = None;
    if let Some(s) = n.scratch.clone() {
        respec(n, &s.0, &s.1);
    }
    let sigs = node(app, id).and_then(|n| n.scratch.clone().map(|s| formulate::signatures(&s.0, &s.1, &n.enc))).unwrap_or_default();
    if !sigs.is_empty() && from != k {
        crate::ai::track(app, EventKind::VisualKindChanged { from: kind_name(from), to: kind_name(k), fields: sigs });
    }
    if from_suggestion {
        crate::ai::track(app, EventKind::Suggestion { arm: "chart".into(), id: format!("chart:{}", kind_name(k)), accepted: true });
    }
    app.ai.formulator.loaded = None;
}

fn recommendations(app: &mut VizApp, ui: &mut Ui, t: &Tokens, id: u64, n: &Node) {
    let Some(s) = &n.scratch else { return };
    let sigs = formulate::signatures(&s.0, &s.1, &n.enc);
    if sigs.is_empty() {
        return;
    }
    let recs = app.ai.brain.recommend_chart(&sigs, 4);
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
        crate::icons::paint(ui.painter(), r, "ai", t.accent);
        ui.label(RichText::new("Recommended for you:").size(11.5).strong());
        for rec in recs {
            let SuggestionAction::ChartType { kind } = &rec.action else { continue };
            let Some(k) = kind_from(kind) else { continue };
            let on = n.chart == k;
            let r = ui.add(egui::Button::selectable(on, RichText::new(format!("{} {:.0}%", rec.title, rec.score * 100.0)).size(11.0))).on_hover_text(&rec.why);
            if r.clicked() && !on {
                set_chart(app, id, k, true);
            }
        }
    });
    if !n.why.is_empty() {
        ui.label(RichText::new(&n.why).size(11.0).color(t.muted));
    }
    if let Some(probs) = &n.decision {
        let txt: Vec<String> = probs.iter().filter(|(_, p)| *p > 0.05).map(|(o, p)| format!("{} {:.0}%", vizardcraft_ai::brain::kind_label(o), p * 100.0)).collect();
        ui.label(RichText::new(format!("Decision model: {}", txt.join(" · "))).size(10.5).color(t.muted));
    }
}

fn shelf(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    egui::Frame::new().fill(t.panel).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8.0).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("Concept encoding").strong().size(12.0));
            ui.label(RichText::new("drag fields here, or type a new field name (e.g. Profit ratio) to derive it").size(10.5).color(t.muted));
        });
        ui.add_space(4.0);
        let unknown = formulate::unknown_fields(&app.project, &app.built, &app.ai.formulator.draft, &[]);
        let cols = 3;
        egui::Grid::new("fm-shelf").num_columns(cols * 2).spacing([8.0, 6.0]).show(ui, |ui| {
            for (i, c) in Encoding::CHANNELS.iter().enumerate() {
                let label = match *c {
                    "x" => "x-axis",
                    "y" => "y-axis",
                    "color" => "color",
                    "size" => "size",
                    "column" => "column",
                    _ => "row",
                };
                ui.label(RichText::new(label).size(11.0).color(t.muted));
                encoding_channel(app, ui, t, c, &unknown);
                if i % cols == cols - 1 {
                    ui.end_row();
                }
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let hint = if app.ai.formulator.selected.is_some() { "Follow up: \"only the top 5\", \"by month instead\", \"show profit ratio too\"…" } else { "Describe a chart: \"profit ratio by region\", \"monthly sales trend\"…" };
            let r = ui.add(egui::TextEdit::singleline(&mut app.ai.formulator.prompt).hint_text(hint).desired_width(ui.available_width() - 150.0));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let busy = app.ai.formulator.job.is_some();
            let label = if app.ai.formulator.selected.is_some() { "Formulate ▸ branch" } else { "Formulate ▸" };
            let clicked = ui.add_enabled(!busy, egui::Button::new(RichText::new(label).color(Color32::WHITE)).fill(t.accent)).on_hover_text("Derive the data and the chart (a new step after the selected one)").clicked();
            if (clicked || enter) && !busy {
                let prompt = std::mem::take(&mut app.ai.formulator.prompt);
                let enc = app.ai.formulator.draft.clone();
                if prompt.trim().is_empty() && enc.is_empty() {
                    app.toast("Add fields to the shelf or describe the chart.");
                } else {
                    let parent = app.ai.formulator.selected;
                    formulate(app, parent, prompt, enc);
                }
            }
        });
    });
}

fn encoding_channel(app: &mut VizApp, ui: &mut Ui, t: &Tokens, c: &str, unknown: &[String]) {
    let frame = egui::Frame::new().fill(t.panel_alt).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6.0).inner_margin(egui::Margin::symmetric(6, 3));
    let (_, dropped) = ui.dnd_drop_zone::<DragName, _>(frame, |ui| {
        ui.set_min_width(160.0);
        ui.horizontal(|ui| {
            let fields = app.ai.formulator.draft.channel(c).clone();
            let mut remove = None;
            for (i, f) in fields.iter().enumerate() {
                let new = unknown.contains(f);
                let (fill, col) = if new { (t.panel, t.accent) } else { (t.accent_soft, t.text) };
                egui::Frame::new().fill(fill).stroke(egui::Stroke::new(1.0, if new { t.accent } else { t.border })).corner_radius(10.0).inner_margin(egui::Margin::symmetric(6, 1)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let shown = f.trim_matches(['[', ']']).to_string();
                        let shown = shown.split_once('[').map(|(_, c)| c.trim_end_matches(']').to_string()).unwrap_or(shown);
                        ui.label(RichText::new(if new { format!("ƒ {shown}") } else { shown }).size(11.0).color(col)).on_hover_text(if new { "New field: it will be derived" } else { f.as_str() });
                        if ui.add(egui::Button::new(RichText::new("✕").size(9.0)).frame(false)).clicked() {
                            remove = Some(i);
                        }
                    });
                });
            }
            if let Some(i) = remove {
                app.ai.formulator.draft.channel_mut(c).remove(i);
            }
            let buf = app.ai.formulator.shelf_input.entry(c.to_string()).or_default();
            let r = ui.add(egui::TextEdit::singleline(buf).hint_text("+ field").desired_width(80.0).frame(egui::Frame::NONE));
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let v = std::mem::take(buf).trim().to_string();
                if !v.is_empty() {
                    app.ai.formulator.draft.channel_mut(c).push(v);
                }
            }
        });
    });
    if let Some(d) = dropped {
        let list = app.ai.formulator.draft.channel_mut(c);
        if !list.contains(&d.0) {
            list.push(d.0.clone());
        }
    }
}

fn empty(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    ui.add_space(30.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("Start a data thread").strong().size(18.0));
        ui.label(RichText::new("Describe a chart, or drag fields onto the shelf. New field names are derived for you.").color(t.muted));
        ui.add_space(12.0);
        for ex in ["Total sales by region", "Profit ratio by category", "Monthly sales trend", "Average order value by channel"] {
            if ui.add(egui::Button::new(RichText::new(ex).size(12.5)).min_size(egui::vec2(260.0, 30.0))).clicked() {
                formulate(app, None, ex.to_string(), Encoding::default());
            }
        }
    });
}

fn details(app: &mut VizApp, ui: &mut Ui, t: &Tokens, n: &Node) {
    ui.horizontal(|ui| {
        for (i, label) in ["Derived data", "Fields & DAX"].iter().enumerate() {
            let on = app.ai.formulator.tab == i;
            let r = ui.add(egui::Button::new(RichText::new(*label).size(12.0).color(if on { t.accent } else { t.text }).strong()).frame(false));
            if on {
                ui.painter().line_segment([r.rect.left_bottom(), r.rect.right_bottom()], egui::Stroke::new(2.0, t.accent));
            }
            if r.clicked() {
                app.ai.formulator.tab = i;
            }
        }
    });
    egui::Frame::new().fill(t.panel).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8.0).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::both().id_salt("fm-details").auto_shrink([false, false]).show(ui, |ui| {
            if app.ai.formulator.tab == 1 {
                if n.derived.is_empty() {
                    ui.label(RichText::new("No derived fields in this step. Type a new field name on the shelf (e.g. \"Profit ratio\") to derive one.").color(t.muted).size(11.5));
                }
                for d in &n.derived {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("ƒ {}", d.name)).strong().size(12.0));
                        ui.label(RichText::new(if d.kind == "column" { "calculated column" } else { "measure" }).size(10.5).color(t.muted));
                        match &d.error {
                            Some(e) => ui.label(RichText::new(format!("✕ {e}")).size(11.0).color(t.danger)),
                            None => ui.label(RichText::new("✓ validated").size(11.0).color(t.good)),
                        };
                    });
                    ui.label(RichText::new(&d.dax).monospace().size(11.0));
                    ui.add_space(4.0);
                }
                ui.label(RichText::new(format!("Encoding: {}", serde_json::to_string(&n.enc).unwrap_or_default())).size(10.5).color(t.muted));
            } else if let Some(d) = &n.data {
                if let Some(e) = &d.error {
                    ui.label(RichText::new(e).color(t.danger));
                    return;
                }
                // The same field on two channels (x and color) is one column of data: show it once.
                let shown: Vec<usize> = (0..d.cols.len()).filter(|&c| !d.cols[..c].iter().any(|p| p.label == d.cols[c].label)).collect();
                egui::Grid::new("fm-data-grid").striped(true).spacing([14.0, 3.0]).show(ui, |ui| {
                    for &c in &shown {
                        ui.label(RichText::new(&d.cols[c].label).strong().size(11.0));
                    }
                    ui.end_row();
                    for r in 0..d.rows.len().min(200) {
                        for &c in &shown {
                            ui.label(RichText::new(d.text(r, c)).size(11.0));
                        }
                        ui.end_row();
                    }
                });
            }
        });
    });
}
