//! Typed decisions: "which chart type", "is this column an ID", "how confident"… answered with a
//! probability for every option.
//!
//! Three backends share the [`Decide`] trait:
//! - [`OpenJevDecider`]: the OpenJEV decision model's helper (`POST /v1/systemone`): one forward
//!   pass per question, calibrated probabilities.
//! - [`LlmDecider`]: any chat provider, asked for a constrained JSON object of probabilities.
//! - [`LocalDecider`]: on device, no network: learned priors from the personal brain plus lexical
//!   evidence from the state text. Always available; the fallback for the other two.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::AiError;
use crate::provider::{ChatProvider, ChatRequest, Message};
use crate::settings::OpenJevConfig;

#[derive(Clone, Debug, PartialEq)]
pub enum QKind {
    /// Pick one option (name, optional description).
    Choice(Vec<(String, Option<String>)>),
    /// Yes / no.
    Noul,
    /// Ordered levels, lowest first.
    Score(Vec<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Question {
    pub key: String,
    pub instructions: String,
    pub kind: QKind,
}

impl Question {
    pub fn choice(key: &str, instructions: &str, options: &[&str]) -> Question {
        Question { key: key.into(), instructions: instructions.into(), kind: QKind::Choice(options.iter().map(|o| (o.to_string(), None)).collect()) }
    }
    pub fn noul(key: &str, instructions: &str) -> Question {
        Question { key: key.into(), instructions: instructions.into(), kind: QKind::Noul }
    }
    pub fn score(key: &str, instructions: &str, levels: &[&str]) -> Question {
        Question { key: key.into(), instructions: instructions.into(), kind: QKind::Score(levels.iter().map(|s| s.to_string()).collect()) }
    }
    /// The option names (yes/no for a noul question, the levels for a score).
    pub fn options(&self) -> Vec<String> {
        match &self.kind {
            QKind::Choice(o) => o.iter().map(|(n, _)| n.clone()).collect(),
            QKind::Noul => vec!["yes".into(), "no".into()],
            QKind::Score(l) => l.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Answer {
    pub key: String,
    /// Probability per option, in option order.
    pub probs: Vec<(String, f64)>,
    /// The most likely option.
    pub choice: String,
    /// 0 (uniform) … 1 (certain).
    pub confidence: f64,
    /// Yes probability for a noul question.
    pub yes: Option<f64>,
    /// Expected level index for a score question.
    pub score: Option<f64>,
}

impl Answer {
    pub fn prob(&self, option: &str) -> f64 {
        self.probs.iter().find(|(o, _)| o == option).map(|(_, p)| *p).unwrap_or(0.0)
    }

    /// Builds an answer from (unnormalized) weights.
    pub fn from_weights(q: &Question, weights: &[f64]) -> Answer {
        let opts = q.options();
        let total: f64 = weights.iter().filter(|w| w.is_finite() && **w > 0.0).sum();
        let n = opts.len().max(1) as f64;
        let probs: Vec<(String, f64)> = opts
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let w = weights.get(i).copied().filter(|w| w.is_finite() && *w > 0.0).unwrap_or(0.0);
                (o.clone(), if total > 0.0 { w / total } else { 1.0 / n })
            })
            .collect();
        finish(q, probs)
    }
}

fn finish(q: &Question, probs: Vec<(String, f64)>) -> Answer {
    let n = probs.len().max(1) as f64;
    let (choice, pmax) = probs.iter().fold((String::new(), -1.0), |acc, (o, p)| if *p > acc.1 { (o.clone(), *p) } else { acc });
    let confidence = if n > 1.0 { ((pmax - 1.0 / n) / (1.0 - 1.0 / n)).clamp(0.0, 1.0) } else { 1.0 };
    let yes = matches!(q.kind, QKind::Noul).then(|| probs.first().map(|(_, p)| *p).unwrap_or(0.5));
    let score = matches!(q.kind, QKind::Score(_)).then(|| probs.iter().enumerate().map(|(i, (_, p))| i as f64 * p).sum());
    Answer { key: q.key.clone(), probs, choice, confidence, yes, score }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decision {
    pub answers: Vec<Answer>,
    pub backend: String,
    pub latency_ms: f64,
    pub input_tokens: u64,
}

impl Decision {
    pub fn get(&self, key: &str) -> Option<&Answer> {
        self.answers.iter().find(|a| a.key == key)
    }
}

pub trait Decide: Send + Sync {
    fn name(&self) -> String;
    fn decide(&self, state: &str, questions: &[Question]) -> Result<Decision, AiError>;
}

// ============================================================== OpenJEV

pub struct OpenJevDecider {
    pub cfg: OpenJevConfig,
}

/// The `/v1/systemone` request body.
pub fn systemone_body(state: &str, questions: &[Question]) -> Value {
    let mut qs = serde_json::Map::new();
    for q in questions {
        let v = match &q.kind {
            QKind::Choice(opts) => {
                let crit: serde_json::Map<String, Value> = opts.iter().map(|(n, d)| (n.clone(), d.clone().map(Value::String).unwrap_or(Value::Null))).collect();
                json!({"type": "choice", "instructions": q.instructions, "criteria": crit})
            }
            QKind::Noul => json!({"type": "noul", "instructions": q.instructions}),
            QKind::Score(levels) => json!({"type": "score", "instructions": q.instructions, "criteria": levels}),
        };
        qs.insert(q.key.clone(), v);
    }
    json!({"model": "openjev", "state": state, "questions": qs})
}

/// Parses a `/v1/systemone` answer (shape documented in OpenJEV's `serve/SERVE.md`).
pub fn parse_systemone(v: &Value, questions: &[Question]) -> Result<Vec<Answer>, AiError> {
    if let Some(e) = v.get("error") {
        return Err(AiError::BadResponse(e.get("message").and_then(Value::as_str).unwrap_or("error").to_string()));
    }
    let answers = v.get("answers").ok_or_else(|| AiError::BadResponse("no `answers` in the OpenJEV response".into()))?;
    let mut out = Vec::new();
    for q in questions {
        let Some(a) = answers.get(&q.key) else { return Err(AiError::BadResponse(format!("no answer for `{}`", q.key))) };
        let probs: Vec<(String, f64)> = match &q.kind {
            QKind::Noul => {
                let y = a.get("noul").and_then(Value::as_f64).ok_or_else(|| AiError::BadResponse("noul answer without a probability".into()))?.clamp(0.0, 1.0);
                vec![("yes".into(), y), ("no".into(), 1.0 - y)]
            }
            QKind::Choice(opts) => opts.iter().map(|(n, _)| (n.clone(), a.pointer(&format!("/probabilities/{}", n.replace('~', "~0").replace('/', "~1"))).and_then(Value::as_f64).unwrap_or(0.0))).collect(),
            QKind::Score(levels) => levels.iter().enumerate().map(|(i, l)| (l.clone(), a.pointer(&format!("/probabilities/{i}")).and_then(Value::as_f64).unwrap_or(0.0))).collect(),
        };
        let mut ans = finish(q, probs);
        if let Some(c) = a.get("choice").and_then(Value::as_str)
            && q.options().iter().any(|o| o == c)
        {
            ans.choice = c.to_string();
        }
        if let Some(c) = a.get("confidence").and_then(Value::as_f64) {
            ans.confidence = c.clamp(0.0, 1.0);
        }
        if let Some(s) = a.get("score").and_then(Value::as_f64) {
            ans.score = Some(s);
            if let QKind::Score(levels) = &q.kind
                && let Some(l) = levels.get(s.round().max(0.0) as usize)
                && ans.probs.iter().all(|(_, p)| *p == 0.0)
            {
                ans.choice = l.clone();
            }
        }
        out.push(ans);
    }
    Ok(out)
}

impl Decide for OpenJevDecider {
    fn name(&self) -> String {
        format!("OpenJEV @ {}", self.cfg.base_url)
    }
    fn decide(&self, state: &str, questions: &[Question]) -> Result<Decision, AiError> {
        let t0 = Instant::now();
        let url = format!("{}/v1/systemone", self.cfg.base_url.trim_end_matches('/'));
        let mut headers = vec![];
        if !self.cfg.token.is_empty() {
            headers.push(("Authorization".to_string(), format!("Bearer {}", self.cfg.token)));
        }
        let v = crate::http::post_json(&url, &headers, &systemone_body(state, questions), Duration::from_secs(self.cfg.timeout_secs.max(1)))?;
        let answers = parse_systemone(&v, questions)?;
        Ok(Decision { answers, backend: self.name(), latency_ms: t0.elapsed().as_secs_f64() * 1000.0, input_tokens: v.pointer("/usage/input_tokens").and_then(Value::as_u64).unwrap_or(0) })
    }
}

// ============================================================== LLM emulation

pub struct LlmDecider {
    pub provider: Box<dyn ChatProvider>,
}

pub fn llm_decision_prompt(state: &str, questions: &[Question]) -> String {
    let mut s = String::from(
        "You are a decision model. Read the state and answer every question with a probability for EVERY option.\n\
         Reply with ONLY a JSON object: {\"<question key>\": {\"<option>\": <probability>, ...}, ...}. Probabilities of a question sum to 1.\n\n",
    );
    s.push_str("STATE:\n");
    s.push_str(state);
    s.push_str("\n\nQUESTIONS:\n");
    for q in questions {
        s.push_str(&format!("- key `{}`: {} Options: {}\n", q.key, q.instructions, q.options().iter().map(|o| format!("\"{o}\"")).collect::<Vec<_>>().join(", ")));
    }
    s
}

impl Decide for LlmDecider {
    fn name(&self) -> String {
        format!("LLM decisions · {}", self.provider.label())
    }
    fn decide(&self, state: &str, questions: &[Question]) -> Result<Decision, AiError> {
        let t0 = Instant::now();
        let req = ChatRequest { messages: vec![Message::user(llm_decision_prompt(state, questions))], tools: vec![], temperature: Some(0.0), max_tokens: 600, json_mode: true };
        let r = self.provider.chat(&req)?;
        let v = crate::extract_json(&r.content).ok_or_else(|| AiError::BadResponse("the model didn't answer with JSON".into()))?;
        let answers = questions
            .iter()
            .map(|q| {
                let opts = q.options();
                let a = v.get(&q.key);
                let w: Vec<f64> = opts
                    .iter()
                    .map(|o| match a {
                        Some(Value::Object(m)) => m.iter().find(|(k, _)| k.eq_ignore_ascii_case(o)).and_then(|(_, p)| p.as_f64()).unwrap_or(0.0),
                        Some(Value::String(s)) if s.eq_ignore_ascii_case(o) => 1.0,
                        Some(Value::Bool(b)) => f64::from(u8::from((*b && o == "yes") || (!*b && o == "no"))),
                        _ => 0.0,
                    })
                    .collect();
                Answer::from_weights(q, &w)
            })
            .collect();
        Ok(Decision { answers, backend: self.name(), latency_ms: t0.elapsed().as_secs_f64() * 1000.0, input_tokens: r.usage.input_tokens })
    }
}

// ============================================================== local

/// On-device decisions: Dirichlet-smoothed priors learned from past answers to the same question
/// key (from the brain), combined with lexical overlap between the state and each option.
#[derive(Clone, Debug, Default)]
pub struct LocalDecider {
    /// question key → option → how often the user ended up choosing it.
    pub priors: BTreeMap<String, BTreeMap<String, f64>>,
}

pub fn tokens(s: &str) -> Vec<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 1).map(str::to_string).collect()
}

impl Decide for LocalDecider {
    fn name(&self) -> String {
        "Personal brain (on device)".into()
    }
    fn decide(&self, state: &str, questions: &[Question]) -> Result<Decision, AiError> {
        let t0 = Instant::now();
        let st: std::collections::HashSet<String> = tokens(state).into_iter().collect();
        let answers = questions
            .iter()
            .map(|q| {
                let prior = self.priors.get(&q.key);
                let descs: Vec<String> = match &q.kind {
                    QKind::Choice(o) => o.iter().map(|(n, d)| format!("{n} {}", d.clone().unwrap_or_default())).collect(),
                    _ => q.options(),
                };
                let w: Vec<f64> = q
                    .options()
                    .iter()
                    .zip(descs.iter())
                    .map(|(o, d)| {
                        let p = 1.0 + prior.and_then(|m| m.get(o)).copied().unwrap_or(0.0);
                        let overlap = tokens(d).iter().filter(|t| st.contains(*t)).count() as f64;
                        p * (1.0 + overlap)
                    })
                    .collect();
                Answer::from_weights(q, &w)
            })
            .collect();
        Ok(Decision { answers, backend: self.name(), latency_ms: t0.elapsed().as_secs_f64() * 1000.0, input_tokens: 0 })
    }
}

/// Tries `primary`, falling back to `fallback` when it fails (server down, bad answer).
pub fn decide_with_fallback(primary: &dyn Decide, fallback: &dyn Decide, state: &str, questions: &[Question]) -> (Decision, Option<AiError>) {
    match primary.decide(state, questions) {
        Ok(d) => (d, None),
        Err(e) => match fallback.decide(state, questions) {
            Ok(d) => (d, Some(e)),
            Err(e2) => (Decision { answers: questions.iter().map(|q| Answer::from_weights(q, &[])).collect(), backend: "uniform".into(), ..Decision::default() }, Some(e2)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::mock;
    use crate::provider::{ChatResponse, ScriptedProvider};

    fn questions() -> Vec<Question> {
        vec![
            Question::choice("chart", "Which chart type fits best?", &["line", "bar", "pie"]),
            Question::noul("is_id", "Is the column `CustomerID` an identifier?"),
            Question::score("urgency", "How urgent?", &["low", "mid", "high"]),
        ]
    }

    #[test]
    fn openjev_request_shape_and_parsing() {
        let answer = json!({
            "id": "shim-1", "model": "openjev T=0.85",
            "answers": {
                "chart": {"type": "choice", "choice": "line", "probabilities": {"line": 0.7, "bar": 0.2, "pie": 0.1}, "confidence": 0.55},
                "is_id": {"type": "noul", "noul": 0.93},
                "urgency": {"type": "score", "score": 1.4, "legend": {"0": "low", "1": "mid", "2": "high"}, "probabilities": {"0": 0.1, "1": 0.4, "2": 0.5}, "confidence": 0.3}
            },
            "usage": {"input_tokens": 321, "output_tokens": 0}
        });
        let m = mock::serve(vec![(200, answer.to_string())]);
        let d = OpenJevDecider { cfg: OpenJevConfig { base_url: m.url.clone(), token: "tok".into(), timeout_secs: 5 } };
        let r = d.decide("Fields: Date (date), Sales (measure)", &questions()).unwrap();
        assert_eq!(r.input_tokens, 321);
        let chart = r.get("chart").unwrap();
        assert_eq!(chart.choice, "line");
        assert!((chart.prob("bar") - 0.2).abs() < 1e-9);
        assert!((chart.confidence - 0.55).abs() < 1e-9);
        assert!((r.get("is_id").unwrap().yes.unwrap() - 0.93).abs() < 1e-9);
        assert_eq!(r.get("is_id").unwrap().choice, "yes");
        let u = r.get("urgency").unwrap();
        assert_eq!(u.choice, "high");
        assert!((u.score.unwrap() - 1.4).abs() < 1e-9);
        let rec = m.received.lock().unwrap();
        assert_eq!(rec[0].path, "/v1/systemone");
        assert!(rec[0].headers.iter().any(|(k, v)| k == "authorization" && v == "Bearer tok"));
        let body: Value = serde_json::from_str(&rec[0].body).unwrap();
        assert_eq!(body["model"], "openjev");
        assert_eq!(body["questions"]["chart"]["type"], "choice");
        assert_eq!(body["questions"]["chart"]["criteria"]["line"], Value::Null);
        assert_eq!(body["questions"]["is_id"]["type"], "noul");
        assert!(body["questions"]["is_id"].get("criteria").is_none());
        assert_eq!(body["questions"]["urgency"]["criteria"], json!(["low", "mid", "high"]));
    }

    #[test]
    fn openjev_errors_fall_back() {
        let m = mock::serve(vec![(422, "{\"error\":{\"code\":422,\"message\":\"missing state\"}}".into())]);
        let d = OpenJevDecider { cfg: OpenJevConfig { base_url: m.url.clone(), token: String::new(), timeout_secs: 5 } };
        let (dec, err) = decide_with_fallback(&d, &LocalDecider::default(), "x", &questions());
        assert!(matches!(err, Some(AiError::Http { status: 422, .. })));
        assert_eq!(dec.answers.len(), 3);
        assert!(parse_systemone(&json!({"answers": {}}), &questions()).is_err());
    }

    #[test]
    fn llm_decider_parses_json() {
        let p = ScriptedProvider::new(vec![ChatResponse { content: "```json\n{\"chart\": {\"line\": 0.1, \"bar\": 0.8, \"pie\": 0.1}, \"is_id\": {\"yes\": 0.9, \"no\": 0.1}, \"urgency\": \"low\"}\n```".into(), ..Default::default() }], false);
        let d = LlmDecider { provider: Box::new(p) };
        let r = d.decide("s", &questions()).unwrap();
        assert_eq!(r.get("chart").unwrap().choice, "bar");
        assert_eq!(r.get("is_id").unwrap().choice, "yes");
        assert_eq!(r.get("urgency").unwrap().choice, "low");
    }

    #[test]
    fn local_decider_uses_priors_and_state() {
        let mut d = LocalDecider::default();
        d.priors.entry("chart".into()).or_default().insert("pie".into(), 20.0);
        let r = d.decide("nothing relevant", &questions()).unwrap();
        assert_eq!(r.get("chart").unwrap().choice, "pie");
        let r2 = LocalDecider::default().decide("a bar of something", &questions()).unwrap();
        assert_eq!(r2.get("chart").unwrap().choice, "bar");
    }
}
