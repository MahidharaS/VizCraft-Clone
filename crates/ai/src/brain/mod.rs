//! The personal brain: every installed copy of VizardCraft becomes its own learning model.
//!
//! Like an ad account that learns from every campaign, the brain learns from what this user
//! does: which visuals they build from which kinds of fields, how they format them, which Power
//! Query steps follow which, which DAX patterns they write, which questions they ask and which
//! suggestions they accept. It is:
//!
//! - **local**: an append-only log in `~/.vizardcraft/brain/events.jsonl` and model weights in
//!   `~/.vizardcraft/brain/model.json`; nothing leaves the machine;
//! - **online**: each event updates the learners immediately (no batch training);
//! - **explainable**: every suggestion says why ("you made a line chart for a date and a measure
//!   in 9 of 11 cases");
//! - **opt-in**: learning can be switched off, the log exported, and everything forgotten.

pub mod features;
pub mod learners;
pub mod memory;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use features::FieldSig;
use learners::{Bandit, ChartRecommender, MeasurePatterns, Preferences, Rng, Sequences};
use memory::{Memory, MemoryItem};

use crate::decide::LocalDecider;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum EventKind {
    /// A visual was created or its type chosen for a set of fields. `source`: "gallery",
    /// "copilot", "formulator", "suggestion", "qna".
    VisualCreated { kind: String, fields: Vec<FieldSig>, source: String },
    /// The user changed a visual's type (a strong signal: the new type is preferred).
    VisualKindChanged { from: String, to: String, fields: Vec<FieldSig> },
    FieldAdded { visual: String, role: String, field: FieldSig },
    /// A formatting property set by hand (`scope` = visual kind, "page" or "report").
    Format { scope: String, key: String, value: String },
    Theme { name: String },
    Filter { field: FieldSig, kind: String },
    Measure { name: String, dax: String, format: String },
    Relationship { from: String, to: String },
    Page { kind: String },
    /// A Power Query step (`step` = the step type, e.g. "RemoveColumns") on a source kind.
    Transform { source: String, history: Vec<String>, step: String },
    Source { kind: String },
    /// A question (copilot, Q&A, Formulator) and what resolved it.
    Question { text: String, resolution: String, accepted: bool },
    /// A suggestion shown to the user was accepted or dismissed.
    Suggestion { arm: String, id: String, accepted: bool },
    /// A typed decision (for the local decision backend's priors).
    Decision { key: String, choice: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub ts: f64,
    /// Local hour of day (0–23).
    pub hour: u8,
    #[serde(flatten)]
    pub kind: EventKind,
}

impl Event {
    pub fn now(kind: EventKind) -> Event {
        let now = chrono::Local::now();
        use chrono::Timelike;
        Event { ts: now.timestamp_millis() as f64 / 1000.0, hour: now.hour() as u8, kind }
    }
    pub fn name(&self) -> &'static str {
        match &self.kind {
            EventKind::VisualCreated { .. } => "visual created",
            EventKind::VisualKindChanged { .. } => "visual type changed",
            EventKind::FieldAdded { .. } => "field added",
            EventKind::Format { .. } => "format",
            EventKind::Theme { .. } => "theme",
            EventKind::Filter { .. } => "filter",
            EventKind::Measure { .. } => "measure",
            EventKind::Relationship { .. } => "relationship",
            EventKind::Page { .. } => "page",
            EventKind::Transform { .. } => "transform",
            EventKind::Source { .. } => "data source",
            EventKind::Question { .. } => "question",
            EventKind::Suggestion { .. } => "suggestion",
            EventKind::Decision { .. } => "decision",
        }
    }
}

/// Everything learned, serialized to `model.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrainModel {
    pub version: u32,
    pub charts: ChartRecommender,
    pub prefs: Preferences,
    pub sequences: Sequences,
    pub memory: Memory,
    pub bandit: Bandit,
    pub measures: MeasurePatterns,
    /// Decision key → option → count (priors for the local decider).
    pub decisions: BTreeMap<String, BTreeMap<String, f64>>,
    /// Event counts by type and by hour.
    pub counts: BTreeMap<String, u64>,
    pub hours: Vec<u64>,
    pub events: u64,
    pub first_ts: f64,
    pub last_ts: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum SuggestionAction {
    ChartType { kind: String },
    Format { key: String, value: String },
    Theme { name: String },
    Transform { step: String },
    Measure { name: String, dax: String, format: String },
    Recall { question: String, resolution: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub id: String,
    /// The bandit arm (family): "chart", "format", "theme", "transform", "measure", "recall".
    pub arm: String,
    pub title: String,
    /// Why ("you made a line chart for a date and a measure in 9 of 11 cases").
    pub why: String,
    pub score: f64,
    pub action: SuggestionAction,
}

/// What the user is doing now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SuggestContext {
    pub fields: Vec<FieldSig>,
    pub visual_kind: Option<String>,
    pub source_kind: Option<String>,
    pub last_steps: Vec<String>,
    pub question: Option<String>,
    /// A measure the user is working with and a date column (for measure templates).
    pub measure: Option<String>,
    pub date_column: Option<String>,
    /// Format values already set on the visual (key → value): habits already applied are skipped.
    pub current_format: BTreeMap<String, String>,
    pub max: usize,
}

pub struct Brain {
    pub dir: Option<PathBuf>,
    pub enabled: bool,
    pub model: BrainModel,
    rng: Rng,
    unsaved: usize,
}

impl Brain {
    /// An in-memory brain (tests, or when the home directory isn't writable).
    pub fn in_memory() -> Brain {
        Brain { dir: None, enabled: true, model: BrainModel { version: 1, hours: vec![0; 24], ..BrainModel::default() }, rng: Rng(0x9E37_79B9_7F4A_7C15), unsaved: 0 }
    }

    pub fn default_dir() -> PathBuf {
        crate::home_dir().join("brain")
    }

    /// Opens the brain in `dir`: loads `model.json`, or rebuilds it by replaying `events.jsonl`.
    pub fn open(dir: PathBuf, enabled: bool) -> Brain {
        let mut b = Brain::in_memory();
        b.enabled = enabled;
        b.rng = Rng(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7) | 1);
        if let Some(m) = std::fs::read_to_string(dir.join("model.json")).ok().and_then(|t| serde_json::from_str::<BrainModel>(&t).ok()) {
            b.model = m;
            if b.model.hours.len() != 24 {
                b.model.hours = vec![0; 24];
            }
        } else {
            for e in read_events(&dir.join("events.jsonl")) {
                b.learn(&e);
            }
        }
        b.dir = Some(dir);
        b
    }

    pub fn events_path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join("events.jsonl"))
    }

    /// Records an action: appends it to the log and learns from it (when learning is on).
    pub fn record(&mut self, kind: EventKind) {
        if !self.enabled {
            return;
        }
        let e = Event::now(kind);
        if let Some(p) = self.events_path() {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p)
                && let Ok(line) = serde_json::to_string(&e)
            {
                let _ = writeln!(f, "{line}");
            }
        }
        self.learn(&e);
        self.unsaved += 1;
        if self.unsaved >= 10 {
            self.save();
        }
    }

    /// Online update of every learner from one event.
    pub fn learn(&mut self, e: &Event) {
        let m = &mut self.model;
        m.events += 1;
        if m.first_ts == 0.0 {
            m.first_ts = e.ts;
        }
        m.last_ts = e.ts;
        *m.counts.entry(e.name().to_string()).or_default() += 1;
        if m.hours.len() != 24 {
            m.hours = vec![0; 24];
        }
        if let Some(h) = m.hours.get_mut(usize::from(e.hour.min(23))) {
            *h += 1;
        }
        match &e.kind {
            EventKind::VisualCreated { kind, fields, source } => {
                if !fields.is_empty() {
                    // Charts the AI made count less than the user's own choices.
                    let w = if source == "copilot" { 0.5 } else { 1.0 };
                    m.charts.learn(fields, kind, w);
                }
            }
            EventKind::VisualKindChanged { to, fields, .. } => {
                if !fields.is_empty() {
                    m.charts.learn(fields, to, 1.5);
                }
            }
            EventKind::FieldAdded { .. } | EventKind::Filter { .. } | EventKind::Relationship { .. } | EventKind::Page { .. } => {}
            EventKind::Format { scope, key, value } => m.prefs.learn(scope, key, value),
            EventKind::Theme { name } => m.prefs.learn("report", "theme", name),
            EventKind::Measure { dax, format, .. } => m.measures.learn(dax, format),
            EventKind::Transform { source, history, step } => m.sequences.learn(source, history, step),
            EventKind::Source { kind } => *m.sequences.sources.entry(kind.clone()).or_default() += 1.0,
            EventKind::Question { text, resolution, accepted } => m.memory.add(MemoryItem { question: text.clone(), resolution: resolution.clone(), accepted: *accepted, ts: e.ts, embedding: vec![] }),
            EventKind::Suggestion { arm, accepted, .. } => m.bandit.learn(arm, *accepted),
            EventKind::Decision { key, choice } => *m.decisions.entry(key.clone()).or_default().entry(choice.clone()).or_default() += 1.0,
        }
    }

    pub fn save(&mut self) {
        self.unsaved = 0;
        let Some(dir) = &self.dir else { return };
        let _ = std::fs::create_dir_all(dir);
        if let Ok(t) = serde_json::to_string(&self.model) {
            let tmp = dir.join("model.json.tmp");
            if std::fs::write(&tmp, t).is_ok() {
                let _ = std::fs::rename(&tmp, dir.join("model.json"));
            }
        }
    }

    /// Deletes the log and the learned model.
    pub fn forget(&mut self) {
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_file(dir.join("events.jsonl"));
            let _ = std::fs::remove_file(dir.join("model.json"));
        }
        self.model = Brain::in_memory().model;
        self.unsaved = 0;
    }

    /// Copies the event log to `path`.
    pub fn export_events(&self, path: &std::path::Path) -> Result<usize, String> {
        let events = self.events();
        let text: String = events.iter().filter_map(|e| serde_json::to_string(e).ok()).map(|l| l + "\n").collect();
        std::fs::write(path, text).map_err(|e| e.to_string())?;
        Ok(events.len())
    }

    pub fn events(&self) -> Vec<Event> {
        self.events_path().map(|p| read_events(&p)).unwrap_or_default()
    }

    // ---------------------------------------------------------------- inference

    /// Ranked chart types for fields, with explanations.
    pub fn recommend_chart(&self, fields: &[FieldSig], k: usize) -> Vec<Suggestion> {
        let phrase = features::context_phrase(fields);
        self.model
            .charts
            .rank(fields)
            .into_iter()
            .take(k)
            .map(|c| {
                let label = kind_label(&c.kind);
                let why = if c.personal_n >= 2.0 && c.personal_share > 0.0 {
                    format!("You used a {} for {phrase} {:.0}% of the time ({} of {}).", label.to_lowercase(), c.personal_share * 100.0, (c.personal_share * c.personal_n).round(), c.personal_n.round())
                } else {
                    format!("A good fit for {phrase}.")
                };
                Suggestion { id: format!("chart:{}", c.kind), arm: "chart".into(), title: label, why, score: c.prob, action: SuggestionAction::ChartType { kind: c.kind } }
            })
            .collect()
    }

    /// Suggestions for the current context, ranked by relevance × the bandit's estimate of how
    /// welcome that kind of suggestion is (Thompson sampling: some exploration).
    pub fn suggest(&mut self, ctx: &SuggestContext) -> Vec<Suggestion> {
        let mut out = Vec::new();
        if !ctx.fields.is_empty() {
            let mut charts = self.recommend_chart(&ctx.fields, 3);
            charts.retain(|s| !matches!(&s.action, SuggestionAction::ChartType { kind } if Some(kind) == ctx.visual_kind.as_ref()));
            out.extend(charts.into_iter().take(2));
        }
        if let Some(kind) = &ctx.visual_kind {
            for h in self.model.prefs.habits(kind, 0.65, 3.0) {
                if h.key == "theme" || ctx.current_format.get(&h.key) == Some(&h.value) {
                    continue;
                }
                out.push(Suggestion {
                    id: format!("format:{}={}", h.key, h.value),
                    arm: "format".into(),
                    title: format!("{} {}", key_label(&h.key), value_label(&h.value)),
                    why: format!("You set {} to {} {:.0}% of the time{}.", key_label(&h.key).to_lowercase(), value_label(&h.value), h.share * 100.0, if h.scope == "*" { "" } else { " on this visual type" }),
                    score: h.share * (1.0 - (-h.n / 4.0).exp()),
                    action: SuggestionAction::Format { key: h.key, value: h.value },
                });
            }
        }
        if let Some(src) = &ctx.source_kind {
            for (step, p) in self.model.sequences.predict(src, &ctx.last_steps).into_iter().take(3) {
                out.push(Suggestion { id: format!("transform:{step}"), arm: "transform".into(), title: step.clone(), why: format!("Usually your next step on {src} data ({:.0}%).", p * 100.0), score: p, action: SuggestionAction::Transform { step } });
            }
        }
        if let Some(m) = &ctx.measure {
            for (pattern, n) in self.model.measures.top().into_iter().take(3) {
                if n < 2.0 {
                    continue;
                }
                if let Some((name, dax, format)) = self.model.measures.template(&pattern, m, ctx.date_column.as_deref()) {
                    out.push(Suggestion { id: format!("measure:{name}"), arm: "measure".into(), title: format!("New measure: {name}"), why: format!("You often write {pattern} measures ({n:.0} so far)."), score: (n / 10.0).min(0.9), action: SuggestionAction::Measure { name, dax, format } });
                }
            }
        }
        if let Some(q) = &ctx.question {
            for (it, s) in self.model.memory.recall(q, 2, None) {
                out.push(Suggestion {
                    id: format!("recall:{}", it.question),
                    arm: "recall".into(),
                    title: format!("Like \"{}\"", it.question),
                    why: format!("You asked something similar before: {}", it.resolution.chars().take(80).collect::<String>()),
                    score: (s / 10.0).min(0.95),
                    action: SuggestionAction::Recall { question: it.question, resolution: it.resolution },
                });
            }
        }
        for s in out.iter_mut() {
            let theta = self.model.bandit.sample(&s.arm, &mut self.rng);
            s.score *= 0.5 + theta;
        }
        out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        out.truncate(if ctx.max == 0 { 4 } else { ctx.max });
        out
    }

    /// A short description of this user's habits, added to the copilot's system prompt so a
    /// cloud or local LLM personalises its answers too.
    pub fn profile_summary(&self) -> String {
        let m = &self.model;
        if m.events < 3 {
            return String::new();
        }
        let mut lines = vec![format!("About this user (learned on device from {} actions):", m.events)];
        let mut by_ctx: Vec<(String, String, f64, f64)> = m
            .charts
            .counts
            .iter()
            .filter_map(|(ctx, c)| {
                let n: f64 = c.values().sum();
                let (k, v) = c.iter().max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))?;
                (n >= 2.0).then(|| (ctx.clone(), k.clone(), v / n, n))
            })
            .collect();
        by_ctx.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
        for (ctx, k, share, n) in by_ctx.iter().take(4) {
            lines.push(format!("- For {} they usually build a {} ({:.0}% of {n:.0}).", ctx.replace('+', " + "), kind_label(k).to_lowercase(), share * 100.0));
        }
        for h in m.prefs.habits("*", 0.65, 3.0).into_iter().take(5) {
            lines.push(format!("- Formatting habit: {} = {}.", h.key, h.value));
        }
        for h in m.prefs.habits("report", 0.6, 2.0).into_iter().filter(|h| h.key == "theme") {
            lines.push(format!("- Preferred theme: {}.", h.value));
        }
        let top: Vec<String> = m.measures.top().into_iter().take(3).map(|(p, n)| format!("{p} ({n:.0})")).collect();
        if !top.is_empty() {
            lines.push(format!("- DAX patterns they write: {}.", top.join(", ")));
        }
        let mut src: Vec<(&String, &f64)> = m.sequences.sources.iter().collect();
        src.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
        if !src.is_empty() {
            lines.push(format!("- Data sources they use most: {}.", src.iter().take(3).map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(", ")));
        }
        lines.push("Follow these habits when the user doesn't say otherwise.".into());
        lines.join("\n")
    }

    /// The on-device decision backend, primed with this user's past choices.
    pub fn decider(&self) -> LocalDecider {
        let mut priors = self.model.decisions.clone();
        // Chart-type questions also use the recommender's counts.
        let chart: BTreeMap<String, f64> = self.model.charts.counts.values().flat_map(|m| m.iter()).fold(BTreeMap::new(), |mut acc, (k, v)| {
            *acc.entry(k.clone()).or_default() += v;
            acc
        });
        priors.entry("chart_type".into()).or_default().extend(chart);
        LocalDecider { priors }
    }

    /// How sure the brain is about this user so far (0–1): grows with evidence.
    pub fn maturity(&self) -> f64 {
        1.0 - (-(self.model.events as f64) / 150.0).exp()
    }
}

pub fn read_events(path: &std::path::Path) -> Vec<Event> {
    std::fs::read_to_string(path).map(|t| t.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()).unwrap_or_default()
}

/// "line" → "Line chart".
pub fn kind_label(serde_name: &str) -> String {
    serde_json::from_value::<vizardcraft_core::doc::VisualKind>(serde_json::Value::String(serde_name.to_string())).map(|k| k.label().to_string()).unwrap_or_else(|_| serde_name.to_string())
}

fn key_label(key: &str) -> String {
    match key {
        "labels.show" => "Data labels".into(),
        "legend.show" => "Legend".into(),
        "legend.position" => "Legend position".into(),
        "title.show" => "Title".into(),
        "title.bold" => "Bold title".into(),
        "title.size" => "Title size".into(),
        "title.align" => "Title alignment".into(),
        "border.show" => "Border".into(),
        "shadow.show" => "Shadow".into(),
        "markers.show" => "Markers".into(),
        "grid.horizontal" => "Gridlines".into(),
        "background.color" => "Background".into(),
        k => k.replace('.', " "),
    }
}

fn value_label(v: &str) -> String {
    match v {
        "true" => "on".into(),
        "false" => "off".into(),
        x => x.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dm() -> Vec<FieldSig> {
        vec![FieldSig::new("date", "Order Date"), FieldSig::new("measure", "Revenue")]
    }

    #[test]
    fn learns_from_synthetic_events_and_persists() {
        let dir = std::env::temp_dir().join(format!("vizardcraft-brain-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut b = Brain::open(dir.clone(), true);
        assert_eq!(b.recommend_chart(&dm(), 1)[0].title, "Line chart");
        for _ in 0..10 {
            b.record(EventKind::VisualCreated { kind: "clusteredColumn".into(), fields: dm(), source: "gallery".into() });
            b.record(EventKind::Format { scope: "clusteredColumn".into(), key: "labels.show".into(), value: "true".into() });
        }
        b.record(EventKind::Theme { name: "Midnight".into() });
        let top = b.recommend_chart(&dm(), 1);
        assert_eq!(top[0].title, "Clustered column chart");
        assert!(top[0].why.contains("100%"), "{}", top[0].why);
        let s = b.suggest(&SuggestContext { fields: dm(), visual_kind: Some("line".into()), ..Default::default() });
        assert!(s.iter().any(|x| matches!(&x.action, SuggestionAction::ChartType { kind } if kind == "clusteredColumn")));
        assert!(s.iter().any(|x| matches!(&x.action, SuggestionAction::Format { key, value } if key == "labels.show" && value == "true")), "{s:?}");
        assert!(b.profile_summary().contains("clustered column"), "{}", b.profile_summary());
        b.save();
        // A fresh instance reads the model back; with the model gone it replays the log.
        let b2 = Brain::open(dir.clone(), true);
        assert_eq!(b2.model.events, 21);
        let _ = std::fs::remove_file(dir.join("model.json"));
        let b3 = Brain::open(dir.clone(), true);
        assert_eq!(b3.model.events, 21);
        assert_eq!(b3.recommend_chart(&dm(), 1)[0].title, "Clustered column chart");
        let mut b4 = b3;
        b4.forget();
        assert_eq!(b4.model.events, 0);
        assert!(!dir.join("events.jsonl").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disabled_brain_records_nothing() {
        let mut b = Brain::in_memory();
        b.enabled = false;
        b.record(EventKind::Theme { name: "x".into() });
        assert_eq!(b.model.events, 0);
    }

    #[test]
    fn bandit_reorders_suggestion_families() {
        let mut b = Brain::in_memory();
        for _ in 0..6 {
            b.record(EventKind::Format { scope: "line".into(), key: "labels.show".into(), value: "true".into() });
        }
        for _ in 0..30 {
            b.record(EventKind::Suggestion { arm: "format".into(), id: "x".into(), accepted: false });
            b.record(EventKind::Suggestion { arm: "chart".into(), id: "y".into(), accepted: true });
        }
        let ctx = SuggestContext { fields: dm(), visual_kind: Some("table".into()), ..Default::default() };
        let mut chart_first = 0;
        for _ in 0..40 {
            if b.suggest(&ctx).first().map(|s| s.arm.as_str()) == Some("chart") {
                chart_first += 1;
            }
        }
        assert!(chart_first > 30, "{chart_first}");
    }

    #[test]
    fn transform_and_measure_and_memory_suggestions() {
        let mut b = Brain::in_memory();
        for _ in 0..3 {
            b.record(EventKind::Transform { source: "csv".into(), history: vec![], step: "PromoteHeaders".into() });
            b.record(EventKind::Measure { name: "m".into(), dax: "DIVIDE([A], [B])".into(), format: "0.0%".into() });
        }
        b.record(EventKind::Question { text: "sales by region".into(), resolution: "column chart".into(), accepted: true });
        let s = b.suggest(&SuggestContext { source_kind: Some("csv".into()), measure: Some("Sales".into()), question: Some("region sales".into()), max: 10, ..Default::default() });
        assert!(s.iter().any(|x| x.arm == "transform"));
        assert!(s.iter().any(|x| x.arm == "measure"));
        assert!(s.iter().any(|x| x.arm == "recall"));
        let d = b.decider();
        assert!(d.priors.contains_key("chart_type"));
    }
}
