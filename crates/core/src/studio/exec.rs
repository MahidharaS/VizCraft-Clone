//! Runs a Studio workflow: every widget's outputs are computed from its inputs and settings and
//! cached under a hash of (kind, settings, upstream hashes, data generation), so only what
//! changed re-runs ("auto-run on change"). Errors are captured per widget, never panics.

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use serde_json::Value as J;

use super::eval::{self, EvalResults, Sampling};
use super::learn::{self, Learned, LearnerSpec};
use super::rank;
use super::registry::{self, DriftRow, RegisteredModel};
use super::unsup;
use super::*;
use crate::build::{ColType, TableData};
use crate::ml::{self, Matrix};
use crate::scalar::Scalar;
use crate::stats::{self, Report};

// =============================================================================================
// Values on channels
// =============================================================================================

/// A table with column roles (Orange's "domain"): target, meta attributes, the rest features.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dataset {
    pub table: Rc<TableData>,
    pub target: Option<String>,
    pub metas: Vec<String>,
}

impl Dataset {
    pub fn new(t: TableData) -> Dataset {
        Dataset { table: Rc::new(t), target: None, metas: vec![] }
    }
    /// Same roles, new table (roles of missing columns are dropped).
    pub fn with(&self, t: TableData) -> Dataset {
        let target = self.target.clone().filter(|c| t.column_index(c).is_some());
        let metas = self.metas.iter().filter(|c| t.column_index(c).is_some()).cloned().collect();
        Dataset { table: Rc::new(t), target, metas }
    }
    pub fn features(&self) -> Vec<String> {
        self.table.columns.iter().filter(|c| Some(*c) != self.target.as_ref() && !self.metas.contains(c)).cloned().collect()
    }
    pub fn numeric_features(&self) -> Vec<String> {
        let nums = ml::numeric_columns(&self.table);
        self.features().into_iter().filter(|c| nums.contains(c)).collect()
    }
    pub fn is_classification(&self) -> bool {
        self.target.as_ref().and_then(|t| self.table.column_index(t)).is_some_and(|i| ml::is_categorical(&self.table, i))
    }
    pub fn summary(&self) -> String {
        let t = &self.table;
        let mut s = format!("{} rows · {} columns", fmt_int(t.rows.len()), t.columns.len());
        if let Some(tg) = &self.target {
            s.push_str(&format!(" · target {tg}"));
        }
        s
    }
}

pub fn fmt_int(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A learner, possibly fitted on the data it received.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelOut {
    pub spec: LearnerSpec,
    pub fitted: Option<Rc<Learned>>,
    pub train: Option<Rc<Dataset>>,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Data(Rc<Dataset>),
    Model(Rc<ModelOut>),
    Eval(Rc<EvalResults>),
}

impl Val {
    pub fn data(&self) -> Option<&Rc<Dataset>> {
        if let Val::Data(d) = self { Some(d) } else { None }
    }
    pub fn model(&self) -> Option<&Rc<ModelOut>> {
        if let Val::Model(m) = self { Some(m) } else { None }
    }
    pub fn eval(&self) -> Option<&Rc<EvalResults>> {
        if let Val::Eval(e) = self { Some(e) } else { None }
    }
}

// =============================================================================================
// Views (what a widget window shows, when it needs computed results)
// =============================================================================================

#[derive(Clone, Debug, Default, PartialEq)]
pub enum View {
    #[default]
    None,
    Report(Report),
    Regression { report: Report, fitted: Vec<f64>, resid: Vec<f64> },
    Normality { report: Report, values: Vec<f64> },
    Forecast { report: Report, history: Vec<f64>, fitted: Vec<f64>, forecast: stats::ts::Forecast, labels: Vec<String> },
    Decomposition(stats::ts::Decomposition),
    Acf { acf: Vec<f64>, pacf: Vec<f64>, band: f64, n: usize },
    Distribution { report: Report, values: Vec<f64>, fits: Vec<stats::dist::Fit> },
    Scores { scores: Vec<eval::Scores>, method: String, errors: Vec<(String, String)>, classification: bool },
    Rank(Vec<rank::RankRow>),
    Importance(Vec<(String, f64, f64)>),
    Explain { contributions: Vec<(String, f64)>, prediction: String },
    Dendrogram { merges: Vec<unsup::Merge>, labels: Vec<String>, clusters: Vec<usize>, k: usize },
    Silhouette { values: Vec<f64>, clusters: Vec<usize>, names: Vec<String> },
    Clusters { sizes: Vec<usize>, silhouette: f64, curve: Vec<(usize, f64)> },
    Pca { explained: Vec<f64>, loadings: Matrix, names: Vec<String> },
    Model { summary: String, coefficients: Vec<(String, Vec<f64>)>, classes: Vec<String> },
    Drift(Vec<DriftRow>),
    Text(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Idle,
    Ok,
    Warning,
    Error,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Out {
    pub outputs: Vec<Option<Val>>,
    pub view: View,
    pub state: State,
    /// Error / warning text.
    pub message: String,
    /// One line shown under the node.
    pub summary: String,
    pub ms: f64,
}

impl Out {
    fn err(m: impl Into<String>) -> Out {
        Out { state: State::Error, message: m.into(), ..Default::default() }
    }
    fn idle(m: impl Into<String>) -> Out {
        Out { state: State::Idle, message: m.into(), ..Default::default() }
    }
    fn ok(outputs: Vec<Option<Val>>, view: View, summary: impl Into<String>) -> Out {
        Out { outputs, view, state: State::Ok, summary: summary.into(), ..Default::default() }
    }
}

// =============================================================================================
// Executor
// =============================================================================================

/// What the executor can see of the project.
pub struct Ctx<'a> {
    pub tables: &'a BTreeMap<String, Rc<TableData>>,
    /// Changes whenever the model data changes.
    pub generation: u64,
    pub registry: &'a [RegisteredModel],
    /// Directory relative CSV paths resolve against.
    pub base_dir: Option<std::path::PathBuf>,
}

#[derive(Default)]
pub struct Executor {
    cache: HashMap<u64, (u64, Rc<Out>)>,
}

impl Executor {
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    pub fn get(&self, id: u64) -> Option<Rc<Out>> {
        self.cache.get(&id).map(|c| c.1.clone())
    }

    /// Runs (or reuses) every widget of the workflow.
    pub fn run(&mut self, ctx: &Ctx, wf: &StudioWorkflow) {
        let mut memo = HashMap::new();
        for w in &wf.widgets {
            self.eval(ctx, wf, w.id, &mut memo, 0);
        }
        let ids: Vec<u64> = wf.widgets.iter().map(|w| w.id).collect();
        self.cache.retain(|k, _| ids.contains(k));
    }

    /// Values arriving at a widget, by input port.
    pub fn inputs(&self, wf: &StudioWorkflow, id: u64) -> Vec<(usize, Val)> {
        let mut v = vec![];
        for l in wf.inputs_of(id) {
            if let Some(o) = self.get(l.from)
                && let Some(Some(val)) = o.outputs.get(l.from_port)
            {
                v.push((l.to_port, val.clone()));
            }
        }
        v
    }

    fn eval(&mut self, ctx: &Ctx, wf: &StudioWorkflow, id: u64, memo: &mut HashMap<u64, u64>, depth: usize) -> u64 {
        if let Some(h) = memo.get(&id) {
            return *h;
        }
        let Some(w) = wf.widget(id) else { return 0 };
        if depth > 200 {
            return 0;
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        w.kind.hash(&mut h);
        serde_json::to_string(&w.settings).unwrap_or_default().hash(&mut h);
        let mut links: Vec<&StudioLink> = wf.inputs_of(id);
        links.sort_by_key(|l| (l.to_port, l.from, l.from_port));
        for l in &links {
            let uh = self.eval(ctx, wf, l.from, memo, depth + 1);
            (l.to_port, l.from_port, uh).hash(&mut h);
        }
        if matches!(w.kind.as_str(), "data.table" | "data.csv") {
            ctx.generation.hash(&mut h);
        }
        if w.kind.starts_with("deploy.") {
            ctx.registry.len().hash(&mut h);
            for m in ctx.registry {
                (&m.name, m.version, m.stage.label()).hash(&mut h);
            }
        }
        let hash = h.finish();
        memo.insert(id, hash);
        if self.cache.get(&id).is_some_and(|c| c.0 == hash) {
            return hash;
        }
        let inputs = self.inputs(wf, id);
        let t0 = std::time::Instant::now();
        let mut out = compute(ctx, wf, w, &inputs);
        out.ms = t0.elapsed().as_secs_f64() * 1000.0;
        self.cache.insert(id, (hash, Rc::new(out)));
        hash
    }
}

// =============================================================================================
// Widget computations
// =============================================================================================

fn input_data(inputs: &[(usize, Val)], port: usize) -> Option<Rc<Dataset>> {
    inputs.iter().find(|(p, v)| *p == port && v.data().is_some()).and_then(|(_, v)| v.data().cloned())
}

fn input_models(inputs: &[(usize, Val)], port: usize) -> Vec<Rc<ModelOut>> {
    inputs.iter().filter(|(p, _)| *p == port).filter_map(|(_, v)| v.model().cloned()).collect()
}

fn input_eval(inputs: &[(usize, Val)], port: usize) -> Option<Rc<EvalResults>> {
    inputs.iter().find(|(p, _)| *p == port).and_then(|(_, v)| v.eval().cloned())
}

fn data_out(d: Dataset) -> Option<Val> {
    Some(Val::Data(Rc::new(d)))
}

fn need_data(inputs: &[(usize, Val)]) -> Result<Rc<Dataset>, Out> {
    input_data(inputs, 0).ok_or_else(|| Out::idle("Connect data."))
}

fn compute(ctx: &Ctx, wf: &StudioWorkflow, w: &StudioWidget, inputs: &[(usize, Val)]) -> Out {
    match compute_inner(ctx, wf, w, inputs) {
        Ok(o) => o,
        Err(o) => o,
    }
}

fn e<T>(r: Result<T, String>) -> Result<T, Out> {
    r.map_err(Out::err)
}

/// Columns chosen in a `Columns` setting, or the numeric features.
fn chosen_numeric(d: &Dataset, w: &StudioWidget, key: &str) -> Vec<String> {
    let c = s_list(w, key);
    let nums = ml::numeric_columns(&d.table);
    if c.is_empty() { d.numeric_features() } else { c.into_iter().filter(|x| nums.contains(x)).collect() }
}

/// Numeric matrix with mean imputation; optionally standardized.
fn num_matrix(d: &Dataset, cols: &[String], standardize: bool) -> Result<Matrix, String> {
    if cols.is_empty() {
        return Err("No numeric columns to use.".into());
    }
    let mut x = ml::matrix(&d.table, cols)?;
    ml::impute_mean(&mut x);
    Ok(if standardize { ml::Scaler::fit(&x).apply(&x) } else { x })
}

/// Keeps every `step`-th row so at most `max` remain (deterministic).
fn sample_rows(n: usize, max: usize) -> Vec<usize> {
    if n <= max {
        return (0..n).collect();
    }
    let step = n as f64 / max as f64;
    (0..max).map(|i| ((i as f64) * step) as usize).collect()
}

fn add_meta_cols(d: &Dataset, rows: &[usize], cols: Vec<(String, Vec<Scalar>, ColType)>) -> Dataset {
    let mut t = learn::take_rows(&d.table, rows);
    let mut metas = d.metas.clone();
    for (name, vals, ty) in cols {
        crate::flow::set_column(&mut t, &name, vals, ty);
        if !metas.contains(&name) {
            metas.push(name);
        }
    }
    let mut out = d.with(t);
    out.metas = metas;
    out
}

fn spec_of(w: &StudioWidget) -> LearnerSpec {
    let kind = w.kind.trim_start_matches("model.").to_string();
    let mut params = BTreeMap::new();
    if let Some(t) = widget_type(&w.kind) {
        for p in &t.params {
            if p.key != "name" {
                params.insert(p.key.to_string(), setting(w, p.key));
            }
        }
    }
    let name = s_str(w, "name");
    LearnerSpec { name: if name.is_empty() { w.label.clone() } else { name }, kind, params }
}

fn period_key(serial: f64, period: &str) -> Option<(String, f64)> {
    use chrono::Datelike;
    let d = vizcraft_datamodel::dates::to_date(serial.floor())?;
    Some(match period {
        "day" => (d.format("%Y-%m-%d").to_string(), serial.floor()),
        "week" => {
            let monday = d - chrono::Duration::days(d.weekday().num_days_from_monday() as i64);
            (format!("{} W{:02}", monday.iso_week().year(), monday.iso_week().week()), vizcraft_datamodel::dates::to_serial(monday)?)
        }
        "quarter" => {
            let q = (d.month() - 1) / 3 + 1;
            (format!("{}-Q{q}", d.year()), vizcraft_datamodel::dates::ymd(d.year(), (q - 1) * 3 + 1, 1)?)
        }
        "year" => (d.year().to_string(), vizcraft_datamodel::dates::ymd(d.year(), 1, 1)?),
        _ => (d.format("%Y-%m").to_string(), vizcraft_datamodel::dates::ymd(d.year(), d.month(), 1)?),
    })
}

fn to_serial(v: &Scalar) -> Option<f64> {
    match v {
        Scalar::Num(n) => Some(*n),
        Scalar::Text(s) => {
            let s = s.trim();
            let d = chrono::NaiveDate::parse_from_str(s.get(..10).unwrap_or(s), "%Y-%m-%d").ok().or_else(|| chrono::NaiveDate::parse_from_str(s, "%d/%m/%Y").ok())?;
            vizcraft_datamodel::dates::to_serial(d)
        }
        _ => None,
    }
}

fn series(d: &Dataset, col: &str) -> Result<Vec<f64>, String> {
    let v = stats::column(&d.table, col)?;
    let f: Vec<f64> = v.into_iter().filter(|x| x.is_finite()).collect();
    if f.len() < 4 {
        return Err(format!("`{col}` has fewer than 4 numeric values."));
    }
    Ok(f)
}

fn compute_inner(ctx: &Ctx, wf: &StudioWorkflow, w: &StudioWidget, inputs: &[(usize, Val)]) -> Result<Out, Out> {
    let kind = w.kind.as_str();
    // ------------------------------------------------------------------ sources
    match kind {
        "data.table" => {
            let name = s_str(w, "table");
            if name.is_empty() {
                return Err(Out::idle("Choose a table."));
            }
            let t = ctx.tables.get(&name).or_else(|| ctx.tables.iter().find(|(k, _)| k.eq_ignore_ascii_case(&name)).map(|(_, v)| v)).ok_or_else(|| Out::err(format!("There's no table `{name}` in the model.")))?;
            let d = Dataset { table: t.clone(), target: None, metas: vec![] };
            let s = d.summary();
            return Ok(Out::ok(vec![data_out(d)], View::None, s));
        }
        "data.csv" => {
            let path = s_str(w, "path");
            if path.is_empty() {
                return Err(Out::idle("Choose a CSV file."));
            }
            let mut pb = std::path::PathBuf::from(&path);
            if pb.is_relative()
                && let Some(b) = &ctx.base_dir
            {
                pb = b.join(pb);
            }
            let text = e(std::fs::read_to_string(&pb).map_err(|er| format!("Couldn't read {}: {er}", pb.display())))?;
            let delim = match s_str(w, "delimiter").as_str() {
                "auto" | "" => crate::flow::detect_delimiter(&text),
                d => d.chars().next().unwrap_or(','),
            };
            let rows = crate::flow::parse_csv(&text, delim);
            let name = pb.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "CSV".into());
            let t = crate::flow::table_from_rows(&name, rows, s_bool(w, "header"));
            let d = Dataset::new(t);
            let s = d.summary();
            return Ok(Out::ok(vec![data_out(d)], View::None, s));
        }
        _ => {}
    }
    // ------------------------------------------------------------------ learners
    if kind.starts_with("model.") {
        let spec = spec_of(w);
        let Some(d) = input_data(inputs, 0) else {
            let m = ModelOut { spec, fitted: None, train: None, source: format!("{} › {}", wf.name, w.label) };
            return Ok(Out { outputs: vec![Some(Val::Model(Rc::new(m)))], state: State::Ok, summary: "Learner (no data: not fitted)".into(), ..Default::default() });
        };
        let Some(target) = d.target.clone() else { return Err(Out::err("The data has no target: set one with Select Columns.")) };
        let m = e(learn::fit(&spec, &d.table, &d.features(), &target))?;
        let summary = format!("Fitted on {} rows", fmt_int(m.train_rows));
        let view = View::Model { summary: m.summary(), coefficients: m.coefficients().unwrap_or_default(), classes: m.classes.clone() };
        let mo = ModelOut { spec, fitted: Some(Rc::new(m)), train: Some(d), source: format!("{} › {}", wf.name, w.label) };
        return Ok(Out::ok(vec![Some(Val::Model(Rc::new(mo)))], view, summary));
    }
    // ------------------------------------------------------------------ evaluation-result consumers
    if matches!(kind, "eval.confusion" | "eval.roc" | "eval.lift" | "eval.calibration") {
        let r = input_eval(inputs, 0).ok_or_else(|| Out::idle("Connect evaluation results (from Test & Score)."))?;
        if !r.is_classification() {
            return Err(Out::err("This needs a classification task (a categorical target)."));
        }
        let s = format!("{} learners · {} classes", r.learners.len(), r.classes.len());
        return Ok(Out::ok(vec![], View::None, s));
    }
    if kind == "deploy.register" {
        let m = input_models(inputs, 0).into_iter().next().ok_or_else(|| Out::idle("Connect a model."))?;
        if m.fitted.is_none() {
            return Err(Out::err("The model isn't fitted: connect data to the model widget."));
        }
        let name = s_str(w, "name");
        let versions = ctx.registry.iter().filter(|r| r.name == if name.is_empty() { m.spec.name.clone() } else { name.clone() }).count();
        let s = if versions == 0 { "Ready to register".to_string() } else { format!("{versions} version(s) registered") };
        return Ok(Out::ok(vec![Some(Val::Model(m))], View::None, s));
    }
    // ------------------------------------------------------------------ data consumers
    let d = need_data(inputs).map_err(|o| if widget_type(kind).is_some_and(|t| t.inputs.is_empty()) { Out::err("Unknown widget") } else { o })?;
    let t = &d.table;
    match kind {
        "data.datatable" | "data.feature_stats" => {
            let s = d.summary();
            Ok(Out::ok(vec![Some(Val::Data(d.clone()))], View::None, s))
        }
        "data.select_columns" => {
            let target = s_str(w, "target");
            let ignore = s_list(w, "ignore");
            let metas = s_list(w, "metas");
            let keep: Vec<usize> = (0..t.columns.len()).filter(|i| !ignore.iter().any(|c| c.eq_ignore_ascii_case(&t.columns[*i]))).collect();
            let nt = TableData { name: t.name.clone(), columns: keep.iter().map(|i| t.columns[*i].clone()).collect(), types: keep.iter().map(|i| t.types.get(*i).copied().unwrap_or_default()).collect(), rows: t.rows.iter().map(|r| keep.iter().map(|i| r.get(*i).cloned().unwrap_or(Scalar::Null)).collect()).collect() };
            if !target.is_empty() && nt.column_index(&target).is_none() {
                return Err(Out::err(format!("There's no column `{target}`.")));
            }
            let mut nd = Dataset::new(nt);
            nd.target = if target.is_empty() { d.target.clone().filter(|c| !ignore.contains(c)) } else { Some(target) };
            nd.metas = metas.into_iter().filter(|m| nd.table.column_index(m).is_some() && Some(m) != nd.target.as_ref()).collect();
            let features = nd.features().len();
            let s = format!("{} features{}", features, nd.target.as_ref().map(|t| format!(" · target {t}")).unwrap_or_default());
            Ok(Out::ok(vec![data_out(nd)], View::None, s))
        }
        "data.select_rows" => {
            let c = s_str(w, "column");
            if c.is_empty() {
                return Ok(Out::ok(vec![Some(Val::Data(d.clone())), data_out(d.with(TableData { rows: vec![], ..(**t).clone() }))], View::None, "No condition: all rows"));
            }
            let ci = t.column_index(&c).ok_or_else(|| Out::err(format!("There's no column `{c}`.")))?;
            let op = s_str(w, "op");
            let val = s_str(w, "value");
            let target = crate::sheet::parse_input(&val);
            let (mut yes, mut no) = (vec![], vec![]);
            for (i, r) in t.rows.iter().enumerate() {
                let v = r.get(ci).cloned().unwrap_or(Scalar::Null);
                let ord = crate::scalar::compare(&v, &target);
                let eq = v.text().eq_ignore_ascii_case(&val) || ord == std::cmp::Ordering::Equal && !v.is_null();
                let m = match op.as_str() {
                    "!=" => !eq,
                    "<" => !v.is_null() && ord == std::cmp::Ordering::Less,
                    "<=" => !v.is_null() && ord != std::cmp::Ordering::Greater,
                    ">" => !v.is_null() && ord == std::cmp::Ordering::Greater,
                    ">=" => !v.is_null() && ord != std::cmp::Ordering::Less,
                    "contains" => v.text().to_lowercase().contains(&val.to_lowercase()),
                    "missing" => v.is_null(),
                    "present" => !v.is_null(),
                    _ => eq,
                };
                if m { yes.push(i) } else { no.push(i) }
            }
            let s = format!("{} of {} rows match", fmt_int(yes.len()), fmt_int(t.rows.len()));
            Ok(Out::ok(vec![data_out(d.with(learn::take_rows(t, &yes))), data_out(d.with(learn::take_rows(t, &no)))], View::None, s))
        }
        "data.merge" => {
            let extra = input_data(inputs, 1).ok_or_else(|| Out::idle("Connect the extra data."))?;
            let (lk, rk) = (s_str(w, "left_key"), s_str(w, "right_key"));
            let (lk, rk) = if lk.is_empty() || rk.is_empty() {
                // Guess: the first column name both tables share.
                let common = t.columns.iter().find(|c| extra.table.column_index(c).is_some()).cloned().ok_or_else(|| Out::idle("Choose the key columns."))?;
                (if lk.is_empty() { common.clone() } else { lk }, if rk.is_empty() { common } else { rk })
            } else {
                (lk, rk)
            };
            let how = s_str(w, "how");
            let j = e(crate::flow::join(t, &extra.table, &[lk.clone()], &[rk], if how.is_empty() { "left" } else { &how }))?;
            let s = format!("{} rows · {} columns (on {lk})", fmt_int(j.rows.len()), j.columns.len());
            Ok(Out::ok(vec![data_out(d.with(j))], View::None, s))
        }
        "data.sample" => {
            let n = t.rows.len();
            let mut rng = ml::Rng::new(s_usize(w, "seed") as u64);
            let mode = s_str(w, "mode");
            let (sample, rest): (Vec<usize>, Vec<usize>) = if mode == "bootstrap" {
                let s: Vec<usize> = (0..n).map(|_| rng.below(n)).collect();
                let set: std::collections::HashSet<usize> = s.iter().copied().collect();
                (s, (0..n).filter(|i| !set.contains(i)).collect())
            } else {
                let frac = if mode == "size" { (s_usize(w, "size") as f64 / n.max(1) as f64).min(1.0) } else { s_f64(w, "proportion").clamp(0.0, 1.0) };
                let y = if s_bool(w, "stratify") && d.is_classification() { d.target.as_ref().and_then(|tg| learn::target_data(t, tg, Some(true)).ok()) } else { None };
                let sp = match &y {
                    Some(td) if td.rows.len() == n => eval::splits(n, Some(&td.y), &Sampling::Random { repeats: 1, train_fraction: frac, stratified: true }, rng.next_u64()),
                    _ => eval::splits(n, None, &Sampling::Random { repeats: 1, train_fraction: frac, stratified: false }, rng.next_u64()),
                };
                let (mut a, mut b) = sp.into_iter().next().unwrap_or_default();
                a.sort_unstable();
                b.sort_unstable();
                (a, b)
            };
            let s = format!("{} sampled · {} remaining", fmt_int(sample.len()), fmt_int(rest.len()));
            Ok(Out::ok(vec![data_out(d.with(learn::take_rows(t, &sample))), data_out(d.with(learn::take_rows(t, &rest)))], View::None, s))
        }
        // -------------------------------------------------------------- transforms
        "tf.preprocess" | "tf.impute" => {
            let mut nt = (**t).clone();
            let cols: Vec<String> = if kind == "tf.impute" {
                let c = s_list(w, "columns");
                if c.is_empty() { d.features() } else { c }
            } else {
                chosen_numeric(&d, w, "columns")
            };
            let method = if kind == "tf.impute" { s_str(w, "method") } else { s_str(w, "impute") };
            let idx: Vec<usize> = cols.iter().filter_map(|c| nt.column_index(c)).collect();
            if method == "drop" {
                nt.rows.retain(|r| idx.iter().all(|i| !r.get(*i).is_none_or(Scalar::is_null)));
            } else if method != "none" {
                for &i in &idx {
                    let numeric = !learn::categorical_column(&nt, i);
                    let fill = if numeric {
                        let v: Vec<f64> = nt.rows.iter().filter_map(|r| r.get(i).and_then(Scalar::as_f64)).collect();
                        let s = stats::sorted(&v);
                        Scalar::Num(match method.as_str() {
                            "median" => stats::quantile(&s, 0.5),
                            "constant" => s_f64(w, "value"),
                            _ => stats::mean(&s),
                        })
                    } else {
                        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
                        for r in &nt.rows {
                            if let Some(v) = r.get(i).filter(|v| !v.is_null()) {
                                *counts.entry(v.text()).or_default() += 1;
                            }
                        }
                        counts.into_iter().max_by_key(|x| x.1).map(|x| Scalar::Text(x.0)).unwrap_or(Scalar::Null)
                    };
                    if matches!(fill, Scalar::Num(v) if !v.is_finite()) {
                        continue;
                    }
                    for r in nt.rows.iter_mut() {
                        if let Some(c) = r.get_mut(i)
                            && c.is_null()
                        {
                            *c = fill.clone();
                        }
                    }
                }
            }
            let mut removed = 0;
            if kind == "tf.preprocess" {
                let scale = s_str(w, "scale");
                for &i in &idx {
                    let v: Vec<f64> = nt.rows.iter().filter_map(|r| r.get(i).and_then(Scalar::as_f64)).filter(|x| x.is_finite()).collect();
                    let (lo, hi) = v.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |a, x| (a.0.min(*x), a.1.max(*x)));
                    let (m, sd) = (stats::mean(&v), stats::variance(&v).sqrt());
                    for r in nt.rows.iter_mut() {
                        if let Some(Scalar::Num(x)) = r.get_mut(i) {
                            match scale.as_str() {
                                "standardize" if sd > 0.0 => *x = (*x - m) / sd,
                                "minmax" if hi > lo => *x = (*x - lo) / (hi - lo),
                                _ => {}
                            }
                        }
                    }
                    if let Some(ty) = nt.types.get_mut(i)
                        && scale != "none"
                    {
                        *ty = ColType::Number;
                    }
                }
                if s_bool(w, "drop_constant") {
                    let feats = d.features();
                    let constant: Vec<usize> = feats
                        .iter()
                        .filter_map(|c| nt.column_index(c))
                        .filter(|i| {
                            let first = nt.rows.iter().find_map(|r| r.get(*i).filter(|v| !v.is_null()).cloned());
                            first.is_some_and(|f| nt.rows.iter().all(|r| r.get(*i).is_none_or(|v| v.is_null() || *v == f)))
                        })
                        .collect();
                    removed = constant.len();
                    if !constant.is_empty() {
                        let keep: Vec<usize> = (0..nt.columns.len()).filter(|i| !constant.contains(i)).collect();
                        nt = TableData { name: nt.name.clone(), columns: keep.iter().map(|i| nt.columns[*i].clone()).collect(), types: keep.iter().map(|i| nt.types.get(*i).copied().unwrap_or_default()).collect(), rows: nt.rows.iter().map(|r| keep.iter().map(|i| r.get(*i).cloned().unwrap_or(Scalar::Null)).collect()).collect() };
                    }
                }
            }
            let nd = d.with(nt);
            let s = format!("{}{}", nd.summary(), if removed > 0 { format!(" · {removed} constant removed") } else { String::new() });
            Ok(Out::ok(vec![data_out(nd)], View::None, s))
        }
        "tf.constructor" => {
            let code = s_str(w, "code");
            let mut nt = (**t).clone();
            let mut added = vec![];
            for (ln, line) in code.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let (name, ex) = line.split_once('=').filter(|(a, b)| !a.trim().is_empty() && !b.starts_with('=')).ok_or_else(|| Out::err(format!("Line {}: write `name = expression`.", ln + 1)))?;
                let name = name.trim().trim_matches(|c| c == '[' || c == ']').to_string();
                nt = e(super::expr::add_column(&nt, &name, ex.trim()))?;
                added.push(name);
            }
            let s = format!("+{} columns · {}", added.len(), d.summary());
            Ok(Out::ok(vec![data_out(d.with(nt))], View::None, s))
        }
        "tf.polynomial" => {
            let cols = chosen_numeric(&d, w, "columns");
            let nt = learn::polynomial(t, &cols, s_usize(w, "degree"), s_bool(w, "interactions"));
            let s = format!("+{} columns", nt.columns.len() - t.columns.len());
            Ok(Out::ok(vec![data_out(d.with(nt))], View::None, s))
        }
        "tf.discretize" => {
            let cols = chosen_numeric(&d, w, "columns");
            let bins = s_usize(w, "bins").clamp(2, 20);
            let mut nt = (**t).clone();
            for c in &cols {
                let Some(i) = nt.column_index(c) else { continue };
                let v: Vec<f64> = nt.rows.iter().map(|r| r.get(i).and_then(Scalar::as_f64).unwrap_or(f64::NAN)).collect();
                let s = stats::sorted(&v);
                let (lo, hi) = (s.first().copied().unwrap_or(0.0), s.last().copied().unwrap_or(0.0));
                let mut cuts: Vec<f64> = (1..bins).map(|k| if s_str(w, "method") == "width" { lo + (hi - lo) * k as f64 / bins as f64 } else { stats::quantile(&s, k as f64 / bins as f64) }).collect();
                cuts.dedup();
                let label = |b: usize| {
                    let a = if b == 0 { "<".to_string() } else { format!("{} – ", stats::fmt(cuts[b - 1])) };
                    if b == cuts.len() { format!("≥ {}", stats::fmt(cuts.last().copied().unwrap_or(hi))) } else if b == 0 { format!("< {}", stats::fmt(cuts[0])) } else { format!("{a}{}", stats::fmt(cuts[b])) }
                };
                let vals: Vec<Scalar> = v.iter().map(|x| if x.is_finite() { Scalar::Text(label(cuts.partition_point(|c| c <= x))) } else { Scalar::Null }).collect();
                crate::flow::set_column(&mut nt, c, vals, ColType::Text);
            }
            let s = format!("{} columns discretized", cols.len());
            Ok(Out::ok(vec![data_out(d.with(nt))], View::None, s))
        }
        "tf.timeagg" => {
            let dc = s_str(w, "date");
            let di = t.column_index(&dc).ok_or_else(|| Out::idle("Choose the date column."))?;
            let vc = s_str(w, "value");
            let vi = if vc.is_empty() { None } else { Some(t.column_index(&vc).ok_or_else(|| Out::err(format!("There's no column `{vc}`.")))?) };
            let period = s_str(w, "period");
            let agg = s_str(w, "agg");
            let mut groups: BTreeMap<i64, (String, f64, Vec<f64>)> = BTreeMap::new();
            for r in &t.rows {
                let Some(serial) = r.get(di).and_then(to_serial) else { continue };
                let Some((label, start)) = period_key(serial, &period) else { continue };
                let v = vi.and_then(|i| r.get(i)).and_then(Scalar::as_f64).unwrap_or(1.0);
                groups.entry(start as i64).or_insert_with(|| (label, start, vec![])).2.push(v);
            }
            if groups.is_empty() {
                return Err(Out::err(format!("`{dc}` has no dates.")));
            }
            let name = if vc.is_empty() { "Count".to_string() } else { vc.clone() };
            let rows: Vec<Vec<Scalar>> = groups
                .values()
                .enumerate()
                .map(|(i, (label, start, vals))| {
                    let v = match agg.as_str() {
                        "mean" => stats::mean(vals),
                        "count" => vals.len() as f64,
                        _ => vals.iter().sum(),
                    };
                    vec![Scalar::Text(label.clone()), Scalar::Num(*start), Scalar::Num(i as f64 + 1.0), Scalar::Num(v)]
                })
                .collect();
            let nt = TableData { name: format!("{} by {period}", name), columns: vec!["Period".into(), "Start".into(), "t".into(), name.clone()], types: vec![ColType::Text, ColType::Date, ColType::Integer, ColType::Number], rows };
            let s = format!("{} periods", nt.rows.len());
            let mut nd = Dataset::new(nt);
            nd.target = Some(name);
            nd.metas = vec!["Period".into(), "Start".into()];
            Ok(Out::ok(vec![data_out(nd)], View::None, s))
        }
        // -------------------------------------------------------------- evaluation
        "eval.test_score" => {
            let models = input_models(inputs, 1);
            if models.is_empty() {
                return Err(Out::idle("Connect learners (model widgets)."));
            }
            let target = d.target.clone().ok_or_else(|| Out::err("The data has no target: set one with Select Columns."))?;
            let specs: Vec<LearnerSpec> = models.iter().map(|m| m.spec.clone()).collect();
            let sampling = match s_str(w, "sampling").as_str() {
                "kfold" => Sampling::KFold { k: s_usize(w, "folds"), stratified: false },
                "random" => Sampling::Random { repeats: s_usize(w, "repeats"), train_fraction: s_f64(w, "train_size"), stratified: d.is_classification() },
                "loo" => Sampling::LeaveOneOut,
                "train" => Sampling::TrainOnTest,
                _ => Sampling::KFold { k: s_usize(w, "folds"), stratified: d.is_classification() },
            };
            if matches!(sampling, Sampling::LeaveOneOut) && t.rows.len() > 1500 {
                return Err(Out::err("Leave-one-out on more than 1,500 rows would be very slow: use k-fold."));
            }
            let test = input_data(inputs, 2);
            let r = e(eval::evaluate(&specs, t, &d.features(), &target, &sampling, s_usize(w, "seed") as u64, test.as_ref().map(|x| x.table.as_ref())))?;
            let sc = eval::scores(&r);
            let best = sc.iter().max_by(|a, b| {
                let k = if r.is_classification() { "AUC" } else { "R²" };
                a.get(k).partial_cmp(&b.get(k)).unwrap_or(std::cmp::Ordering::Equal)
            });
            let summary = best.map(|b| if r.is_classification() { format!("Best AUC {:.3} ({})", b.get("AUC"), b.learner) } else { format!("Best R² {:.3} ({})", b.get("R²"), b.learner) }).unwrap_or_default();
            let mut out = Out::ok(vec![Some(Val::Eval(Rc::new(r.clone())))], View::Scores { scores: sc, method: r.method.clone(), errors: r.errors.clone(), classification: r.is_classification() }, summary);
            if !r.errors.is_empty() {
                out.state = State::Warning;
                out.message = r.errors.iter().map(|(a, b)| format!("{a}: {b}")).collect::<Vec<_>>().join("\n");
            }
            Ok(out)
        }
        "eval.predictions" => {
            let models = input_models(inputs, 1);
            if models.is_empty() {
                return Err(Out::idle("Connect fitted models."));
            }
            let mut nt = (**t).clone();
            let mut metas = d.metas.clone();
            for m in &models {
                let Some(f) = &m.fitted else { return Err(Out::err(format!("{} isn't fitted: connect training data to it.", m.spec.name))) };
                let before = nt.columns.len();
                nt = registry::score_table(f, &nt, &f.name(), s_bool(w, "probs"));
                metas.extend(nt.columns[before..].iter().cloned());
            }
            let mut nd = d.with(nt);
            nd.metas = metas;
            let s = format!("{} rows · {} models", fmt_int(t.rows.len()), models.len());
            Ok(Out::ok(vec![data_out(nd)], View::None, s))
        }
        "eval.rank" => {
            let target = d.target.clone().ok_or_else(|| Out::err("Rank needs a target: set one with Select Columns."))?;
            let r = e(rank::rank(t, &d.features(), &target))?;
            let top = s_usize(w, "top");
            let keep: Vec<String> = if top == 0 { d.features() } else { r.iter().take(top).map(|x| x.feature.clone()).collect() };
            let cols: Vec<usize> = (0..t.columns.len()).filter(|i| keep.contains(&t.columns[*i]) || Some(&t.columns[*i]) == d.target.as_ref() || d.metas.contains(&t.columns[*i])).collect();
            let nt = TableData { name: t.name.clone(), columns: cols.iter().map(|i| t.columns[*i].clone()).collect(), types: cols.iter().map(|i| t.types.get(*i).copied().unwrap_or_default()).collect(), rows: t.rows.iter().map(|row| cols.iter().map(|i| row.get(*i).cloned().unwrap_or(Scalar::Null)).collect()).collect() };
            let s = format!("Top: {}", r.first().map(|x| x.feature.as_str()).unwrap_or("—"));
            Ok(Out::ok(vec![data_out(d.with(nt))], View::Rank(r), s))
        }
        "eval.importance" | "eval.explain" => {
            let m = input_models(inputs, 1).into_iter().next().ok_or_else(|| Out::idle("Connect a fitted model."))?;
            let f = m.fitted.clone().ok_or_else(|| Out::err("The model isn't fitted: connect data to it."))?;
            if kind == "eval.importance" {
                let imp = e(rank::permutation_importance(&f, t, s_usize(w, "repeats"), s_usize(w, "seed") as u64))?;
                let s = format!("Most important: {}", imp.first().map(|x| x.0.as_str()).unwrap_or("—"));
                return Ok(Out::ok(vec![], View::Importance(imp), s));
            }
            let row = s_usize(w, "row").min(t.rows.len().saturating_sub(1));
            let one = learn::take_rows(t, &[row]);
            let x = f.encoder.encode(&one).into_iter().next().ok_or_else(|| Out::err("No rows."))?;
            let class = s_usize(w, "class").min(f.classes.len().saturating_sub(1));
            let (v, p) = f.predict_x(&x);
            let pred = if f.is_classifier() { format!("{} (P = {:.3})", f.classes.get(v as usize).cloned().unwrap_or_default(), p.get(v as usize).copied().unwrap_or(0.0)) } else { stats::fmt(v) };
            let c = rank::explain_row(&f, &x, class);
            Ok(Out::ok(vec![], View::Explain { contributions: c, prediction: pred.clone() }, format!("Row {row}: {pred}")))
        }
        // -------------------------------------------------------------- visualisations (drawn by the UI)
        k if k.starts_with("viz.") => {
            let s = d.summary();
            Ok(Out::ok(vec![], View::None, s))
        }
        // -------------------------------------------------------------- unsupervised
        "unsup.kmeans" => {
            let cols = chosen_numeric(&d, w, "columns");
            let x = e(num_matrix(&d, &cols, true))?;
            if x.len() < 3 {
                return Err(Out::err("Too few rows to cluster."));
            }
            let seed = s_usize(w, "seed") as u64;
            let (km, k, curve) = if s_bool(w, "auto") {
                ml::kmeans_auto(&x, 8.min(x.len() - 1), seed)
            } else {
                let k = s_usize(w, "k").clamp(1, x.len());
                (ml::kmeans(&x, k, seed, 300), k, vec![])
            };
            let sil = if x.len() <= 3000 { ml::silhouette(&x, &km.labels) } else { f64::NAN };
            let labels: Vec<Scalar> = km.labels.iter().map(|l| Scalar::Text(format!("C{}", l + 1))).collect();
            let sizes: Vec<usize> = (0..k).map(|c| km.labels.iter().filter(|l| **l == c).count()).collect();
            let rows: Vec<usize> = (0..t.rows.len()).collect();
            let nd = add_meta_cols(&d, &rows, vec![("Cluster".into(), labels, ColType::Text)]);
            Ok(Out::ok(vec![data_out(nd)], View::Clusters { sizes, silhouette: sil, curve }, format!("k = {k} · silhouette {sil:.3}")))
        }
        "unsup.hclust" => {
            let cols = chosen_numeric(&d, w, "columns");
            let rows = sample_rows(t.rows.len(), s_usize(w, "max_rows").max(2));
            let sub = d.with(learn::take_rows(t, &rows));
            let x = e(num_matrix(&sub, &cols, true))?;
            let merges = unsup::linkage(&x, &s_str(w, "linkage"));
            let k = s_usize(w, "k").clamp(1, x.len().max(1));
            let cl = unsup::cut(&merges, x.len(), k);
            let lc = s_str(w, "label");
            let labels: Vec<String> = (0..x.len()).map(|i| if lc.is_empty() { format!("#{}", rows[i] + 1) } else { sub.table.column_index(&lc).map(|c| sub.table.cell(i, c).text()).unwrap_or_default() }).collect();
            let nd = add_meta_cols(&d, &rows, vec![("Cluster".into(), cl.iter().map(|c| Scalar::Text(format!("C{}", c + 1))).collect(), ColType::Text)]);
            Ok(Out::ok(vec![data_out(nd)], View::Dendrogram { merges, labels, clusters: cl, k }, format!("{k} clusters · {} rows", rows.len())))
        }
        "unsup.dbscan" => {
            let cols = chosen_numeric(&d, w, "columns");
            let x = e(num_matrix(&d, &cols, true))?;
            if x.len() > 5000 {
                return Err(Out::err("DBSCAN is limited to 5,000 rows here: sample the data first."));
            }
            let l = ml::dbscan(&x, s_f64(w, "eps"), s_usize(w, "min_pts").max(1));
            let k = l.iter().flatten().max().map(|m| m + 1).unwrap_or(0);
            let noise = l.iter().filter(|v| v.is_none()).count();
            let vals = l.iter().map(|c| Scalar::Text(c.map(|c| format!("C{}", c + 1)).unwrap_or_else(|| "Noise".into()))).collect();
            let rows: Vec<usize> = (0..t.rows.len()).collect();
            let sizes = (0..k).map(|c| l.iter().filter(|v| **v == Some(c)).count()).collect();
            Ok(Out::ok(vec![data_out(add_meta_cols(&d, &rows, vec![("Cluster".into(), vals, ColType::Text)]))], View::Clusters { sizes, silhouette: f64::NAN, curve: vec![] }, format!("{k} clusters · {noise} noise points")))
        }
        "unsup.pca" => {
            let cols = chosen_numeric(&d, w, "columns");
            let x = e(num_matrix(&d, &cols, false))?;
            let k = s_usize(w, "components").clamp(1, cols.len().max(1));
            let all = ml::pca(&x, cols.len(), s_bool(w, "standardize"));
            if all.scores.is_empty() {
                return Err(Out::err("PCA needs at least two rows and one numeric column."));
            }
            let mut nt = TableData { name: format!("{} (PCA)", t.name), columns: vec![], types: vec![], rows: vec![vec![]; t.rows.len()] };
            for c in 0..k {
                nt.columns.push(format!("PC{}", c + 1));
                nt.types.push(ColType::Number);
                for (r, s) in nt.rows.iter_mut().zip(&all.scores) {
                    r.push(Scalar::Num(s.get(c).copied().unwrap_or(0.0)));
                }
            }
            let mut extra: Vec<String> = d.metas.clone();
            if let Some(tg) = &d.target {
                extra.push(tg.clone());
            }
            for c in &extra {
                if let Some(i) = t.column_index(c) {
                    nt.columns.push(c.clone());
                    nt.types.push(t.types.get(i).copied().unwrap_or_default());
                    for (r, src) in nt.rows.iter_mut().zip(&t.rows) {
                        r.push(src.get(i).cloned().unwrap_or(Scalar::Null));
                    }
                }
            }
            let nd = Dataset { table: Rc::new(nt), target: d.target.clone(), metas: d.metas.clone() };
            let cum: f64 = all.explained.iter().take(k).sum();
            Ok(Out::ok(vec![data_out(nd)], View::Pca { explained: all.explained.clone(), loadings: all.loadings.clone(), names: cols }, format!("{k} components · {:.0}% variance", cum * 100.0)))
        }
        "unsup.mds" | "unsup.tsne" => {
            let cols = chosen_numeric(&d, w, "columns");
            let rows = sample_rows(t.rows.len(), s_usize(w, "max_rows").max(4));
            let sub = d.with(learn::take_rows(t, &rows));
            let x = e(num_matrix(&sub, &cols, true))?;
            let seed = s_usize(w, "seed") as u64;
            let (y, names) = if kind == "unsup.mds" { (unsup::mds(&x, 2, seed), ("MDS-x", "MDS-y")) } else { (unsup::tsne(&x, s_f64(w, "perplexity"), s_usize(w, "iterations"), seed), ("t-SNE-x", "t-SNE-y")) };
            let c0 = y.iter().map(|r| Scalar::Num(r.first().copied().unwrap_or(0.0))).collect();
            let c1 = y.iter().map(|r| Scalar::Num(r.get(1).copied().unwrap_or(0.0))).collect();
            let nd = add_meta_cols(&d, &rows, vec![(names.0.into(), c0, ColType::Number), (names.1.into(), c1, ColType::Number)]);
            Ok(Out::ok(vec![data_out(nd)], View::None, format!("{} rows embedded", rows.len())))
        }
        "unsup.silhouette" => {
            let cc = s_str(w, "cluster");
            let ci = t.column_index(&cc).ok_or_else(|| Out::idle("Choose the cluster column."))?;
            let rows = sample_rows(t.rows.len(), s_usize(w, "max_rows").max(2));
            let sub = d.with(learn::take_rows(t, &rows));
            let mut cols = chosen_numeric(&d, w, "columns");
            cols.retain(|c| *c != cc);
            let x = e(num_matrix(&sub, &cols, true))?;
            let mut names: Vec<String> = sub.table.rows.iter().map(|r| r.get(ci).map(Scalar::text).unwrap_or_default()).collect();
            let labels_txt = names.clone();
            names.sort();
            names.dedup();
            let clusters: Vec<usize> = labels_txt.iter().map(|l| names.iter().position(|n| n == l).unwrap_or(0)).collect();
            let vals = unsup::silhouettes(&x, &clusters);
            let avg = stats::mean(&vals);
            Ok(Out::ok(vec![], View::Silhouette { values: vals, clusters, names }, format!("Average silhouette {avg:.3}")))
        }
        // -------------------------------------------------------------- statistics
        "stats.describe" => {
            let cols = chosen_numeric(&d, w, "columns");
            let group = s_str(w, "group");
            let mut rep = Report::new("Descriptive statistics");
            let header: Vec<String> = ["Variable", "Group", "n", "Missing", "Mean", "Std. dev.", "SE", "Min", "Q1", "Median", "Q3", "Max", "Skew", "Kurtosis", "95% CI"].iter().map(|s| s.to_string()).collect();
            let mut rows = vec![];
            let mut out_rows = vec![];
            let groups: Vec<(String, Vec<usize>)> = if group.is_empty() {
                vec![("All".into(), (0..t.rows.len()).collect())]
            } else {
                let gi = t.column_index(&group).ok_or_else(|| Out::err(format!("There's no column `{group}`.")))?;
                let mut m: BTreeMap<String, Vec<usize>> = BTreeMap::new();
                for (i, r) in t.rows.iter().enumerate() {
                    m.entry(r.get(gi).map(Scalar::text).unwrap_or_default()).or_default().push(i);
                }
                m.into_iter().collect()
            };
            for c in &cols {
                let all = e(stats::column(t, c))?;
                for (g, idx) in &groups {
                    let v: Vec<f64> = idx.iter().map(|i| all[*i]).collect();
                    let s = stats::describe(&v);
                    rows.push(vec![c.clone(), g.clone(), s.n.to_string(), s.missing.to_string(), stats::fmt(s.mean), stats::fmt(s.std), stats::fmt(s.se), stats::fmt(s.min), stats::fmt(s.q1), stats::fmt(s.median), stats::fmt(s.q3), stats::fmt(s.max), format!("{:.3}", s.skewness), format!("{:.3}", s.kurtosis), format!("[{}, {}]", stats::fmt(s.ci_low), stats::fmt(s.ci_high))]);
                    out_rows.push(vec![Scalar::Text(c.clone()), Scalar::Text(g.clone()), Scalar::Num(s.n as f64), Scalar::Num(s.mean), Scalar::Num(s.std), Scalar::Num(s.min), Scalar::Num(s.median), Scalar::Num(s.max)]);
                }
            }
            rep.section("Summary").table = Some(stats::RTable { columns: header, rows, p_col: None });
            let nt = TableData { name: "Descriptive statistics".into(), columns: ["Variable", "Group", "n", "Mean", "Std", "Min", "Median", "Max"].iter().map(|s| s.to_string()).collect(), types: vec![ColType::Text, ColType::Text, ColType::Integer, ColType::Number, ColType::Number, ColType::Number, ColType::Number, ColType::Number], rows: out_rows };
            Ok(Out::ok(vec![data_out(Dataset::new(nt))], View::Report(rep), format!("{} variables", cols.len())))
        }
        "stats.ols" | "stats.logit" | "stats.glm" => {
            let target = s_str(w, "target");
            if target.is_empty() {
                return Err(Out::idle("Choose the dependent variable."));
            }
            let mut preds = s_list(w, "predictors");
            if preds.is_empty() {
                preds = d.features().into_iter().filter(|c| *c != target).collect();
            }
            preds.retain(|c| *c != target);
            if kind == "stats.ols" {
                let (fit, des) = e(stats::regression::ols_table(t, &target, &preds))?;
                let rep = stats::regression::ols_report(&fit, &target);
                let mut nt = learn::take_rows(t, &des.rows);
                crate::flow::set_column(&mut nt, "Fitted", fit.fitted.iter().map(|v| Scalar::Num(*v)).collect(), ColType::Number);
                crate::flow::set_column(&mut nt, "Residual", fit.residuals.iter().map(|v| Scalar::Num(*v)).collect(), ColType::Number);
                let s = format!("R² {:.3} · F p {}", fit.r2, stats::fmt_p(fit.f_p));
                return Ok(Out::ok(vec![data_out(d.with(nt))], View::Regression { report: rep, fitted: fit.fitted, resid: fit.residuals }, s));
            }
            if kind == "stats.logit" {
                let ev = s_str(w, "event");
                let des = e(stats::regression::design(t, &target, &preds, true, true, if ev.is_empty() { None } else { Some(ev.as_str()) }))?;
                let mut fit = e(stats::regression::logit(&des.names, &des.x, &des.y))?;
                fit.classes = des.classes.clone();
                let s = format!("Pseudo R² {:.3} · LLR p {}", fit.pseudo_r2, stats::fmt_p(fit.llr_p));
                return Ok(Out::ok(vec![], View::Report(stats::regression::glm_report(&fit, &target)), s));
            }
            let des = e(stats::regression::design(t, &target, &preds, true, false, None))?;
            let fit = e(stats::regression::poisson(&des.names, &des.x, &des.y))?;
            let s = format!("Deviance {}", stats::fmt(fit.deviance));
            Ok(Out::ok(vec![], View::Report(stats::regression::glm_report(&fit, &target)), s))
        }
        "stats.ttest" => {
            use stats::hypothesis as h;
            let c = s_str(w, "column");
            if c.is_empty() {
                return Err(Out::idle("Choose the variable."));
            }
            let test = s_str(w, "test");
            let r = match test.as_str() {
                "one" => {
                    let mu = s_f64(w, "mu");
                    h::one_sample_t(&e(stats::column(t, &c))?, mu).report(&format!("mean of {c} = {}", stats::fmt(mu)))
                }
                "paired" => {
                    let c2 = s_str(w, "column2");
                    if c2.is_empty() {
                        return Err(Out::idle("Choose the second variable for a paired test."));
                    }
                    h::paired_t(&e(stats::column(t, &c))?, &e(stats::column(t, &c2))?).report(&format!("mean of {c} − {c2} = 0"))
                }
                _ => {
                    let g = s_str(w, "group");
                    if g.is_empty() {
                        return Err(Out::idle("Choose the group column."));
                    }
                    let (names, groups) = e(stats::groups(t, &c, &g))?;
                    let pick = |key: &str, default: usize| {
                        let v = s_str(w, key);
                        if v.is_empty() { Some(default) } else { names.iter().position(|n| *n == v) }
                    };
                    let (Some(a), Some(b)) = (pick("a", 0), pick("b", 1)) else { return Err(Out::err("Unknown group name.")) };
                    if a >= groups.len() || b >= groups.len() {
                        return Err(Out::err("The group column needs at least two groups."));
                    }
                    let mut rep = h::two_sample_t(&groups[a], &groups[b], test == "student").report(&format!("mean {c} is equal for {} and {}", names[a], names[b]));
                    if let Some(s) = rep.sections.first_mut() {
                        s.kv.insert(0, ("Groups".into(), format!("{} (n = {}) vs {} (n = {})", names[a], groups[a].len(), names[b], groups[b].len())));
                    }
                    rep
                }
            };
            let p = r.sections.first().and_then(|s| s.kv.iter().find(|(k, _)| k == "p-value")).map(|x| x.1.clone()).unwrap_or_default();
            Ok(Out::ok(vec![], View::Report(r), format!("p = {p}")))
        }
        "stats.anova" => {
            use stats::hypothesis as h;
            let (v, f) = (s_str(w, "value"), s_str(w, "factor"));
            if v.is_empty() || f.is_empty() {
                return Err(Out::idle("Choose the variable and the factor."));
            }
            let f2 = s_str(w, "factor2");
            let mut rep;
            if f2.is_empty() {
                let (names, groups) = e(stats::groups(t, &v, &f))?;
                if groups.len() < 2 {
                    return Err(Out::err("The factor needs at least two groups."));
                }
                let a = h::one_way_anova(&groups);
                rep = h::anova_report(&format!("One-way ANOVA: {v} by {f}"), &a);
                let s = rep.section("Group means");
                s.table = Some(stats::RTable { columns: vec!["Group".into(), "n".into(), "Mean".into(), "Std. dev.".into()], rows: names.iter().zip(&groups).map(|(n, g)| vec![n.clone(), g.len().to_string(), stats::fmt(stats::mean(g)), stats::fmt(stats::variance(g).sqrt())]).collect(), p_col: None });
                if s_bool(w, "tukey") && groups.len() <= 30 {
                    let tk = h::tukey_hsd(&names, &groups);
                    let s = rep.section("Tukey HSD (95% family-wise)");
                    s.table = Some(stats::RTable { columns: ["Group 1", "Group 2", "Mean diff", "Lower", "Upper", "p-adj", "Reject"].iter().map(|s| s.to_string()).collect(), rows: tk.iter().map(|p| vec![p.a.clone(), p.b.clone(), stats::fmt(p.diff), stats::fmt(p.lower), stats::fmt(p.upper), stats::fmt_p(p.p_adj), if p.p_adj < 0.05 { "yes".into() } else { "no".into() }]).collect(), p_col: Some(5) });
                }
            } else {
                let y = e(stats::column(t, &v))?;
                let a: Vec<String> = e(stats::labels(t, &f))?.into_iter().map(|x| x.unwrap_or_default()).collect();
                let b: Vec<String> = e(stats::labels(t, &f2))?.into_iter().map(|x| x.unwrap_or_default()).collect();
                let an = h::two_way_anova(&y, &a, &b, &f, &f2);
                rep = h::anova_report(&format!("Two-way ANOVA: {v} by {f} and {f2}"), &an);
            }
            let p = rep.sections.first().and_then(|s| s.table.as_ref()).and_then(|t| t.rows.first()).and_then(|r| r.get(5)).cloned().unwrap_or_default();
            Ok(Out::ok(vec![], View::Report(rep), format!("p = {p}")))
        }
        "stats.chisq" => {
            let (a, b) = (s_str(w, "a"), s_str(w, "b"));
            if a.is_empty() || b.is_empty() {
                return Err(Out::idle("Choose two categorical columns."));
            }
            let (r, c, o) = stats::hypothesis::crosstab(&e(stats::labels(t, &a))?, &e(stats::labels(t, &b))?);
            if r.len() < 2 || c.len() < 2 || r.len() * c.len() > 2500 {
                return Err(Out::err("Each column needs between 2 and 50 categories."));
            }
            let ct = stats::hypothesis::chi2_independence(r, c, o);
            let mut rep = Report::new(format!("χ² test of independence: {a} × {b}"));
            let s = rep.section("Result");
            s.kv("χ²", stats::fmt(ct.chi2)).kv("Degrees of freedom", stats::fmt(ct.df)).kv("p-value", stats::fmt_p(ct.p)).kv("Cramér's V", format!("{:.3}", ct.cramers_v));
            s.note = stats::hypothesis::conclusion(ct.p, &format!("{a} and {b} are independent"));
            if ct.expected.iter().flatten().any(|e| *e < 5.0) {
                s.note.push_str(" Some expected counts are below 5: the χ² approximation may be poor.");
            }
            let s = rep.section("Observed (expected) counts");
            let mut cols = vec![format!("{a} \\ {b}")];
            cols.extend(ct.cols.iter().cloned());
            s.table = Some(stats::RTable { columns: cols, rows: ct.rows.iter().enumerate().map(|(i, rn)| std::iter::once(rn.clone()).chain(ct.observed[i].iter().zip(&ct.expected[i]).map(|(o, e)| format!("{o:.0} ({e:.1})"))).collect()).collect(), p_col: None });
            Ok(Out::ok(vec![], View::Report(rep), format!("p = {}", stats::fmt_p(ct.p))))
        }
        "stats.correlation" => {
            let (x, y) = (s_str(w, "x"), s_str(w, "y"));
            if x.is_empty() || y.is_empty() {
                return Err(Out::idle("Choose two variables."));
            }
            let (a, b) = (e(stats::column(t, &x))?, e(stats::column(t, &y))?);
            let r = match s_str(w, "method").as_str() {
                "spearman" => stats::hypothesis::spearman(&a, &b),
                "kendall" => {
                    if a.len() > 5000 {
                        return Err(Out::err("Kendall's τ is limited to 5,000 rows here."));
                    }
                    stats::hypothesis::kendall(&a, &b)
                }
                _ => stats::hypothesis::pearson(&a, &b),
            };
            let s = format!("{} = {:.3}, p = {}", r.statistic_name, r.statistic, stats::fmt_p(r.p_value));
            Ok(Out::ok(vec![], View::Report(r.report(&format!("no correlation between {x} and {y}"))), s))
        }
        "stats.normality" => {
            let c = s_str(w, "column");
            if c.is_empty() {
                return Err(Out::idle("Choose the variable."));
            }
            let v = e(stats::column(t, &c))?;
            let sw = stats::hypothesis::shapiro_wilk(&v);
            let jb = stats::hypothesis::jarque_bera(&v);
            let mut rep = Report::new(format!("Normality of {c}"));
            for r in [&sw, &jb] {
                let s = rep.section(r.name.clone());
                s.kv(r.statistic_name.clone(), stats::fmt(r.statistic)).kv("p-value", stats::fmt_p(r.p_value));
                s.note = stats::hypothesis::conclusion(r.p_value, &format!("{c} is normally distributed"));
            }
            let d0 = stats::describe(&v);
            rep.section("Shape").kv("Skewness", format!("{:.3}", d0.skewness)).kv("Excess kurtosis", format!("{:.3}", d0.kurtosis)).kv("n", d0.n.to_string());
            Ok(Out::ok(vec![], View::Normality { report: rep, values: stats::sorted(&v) }, format!("Shapiro–Wilk p = {}", stats::fmt_p(sw.p_value))))
        }
        "stats.nonparam" => {
            let (v, g) = (s_str(w, "value"), s_str(w, "group"));
            if v.is_empty() || g.is_empty() {
                return Err(Out::idle("Choose the variable and the groups."));
            }
            let (names, groups) = e(stats::groups(t, &v, &g))?;
            if groups.len() < 2 {
                return Err(Out::err("Need at least two groups."));
            }
            let r = if groups.len() == 2 { stats::hypothesis::mann_whitney(&groups[0], &groups[1]) } else { stats::hypothesis::kruskal_wallis(&groups) };
            let mut rep = r.report(&format!("{v} has the same distribution in every group of {g}"));
            let s = rep.section("Groups");
            s.table = Some(stats::RTable { columns: vec!["Group".into(), "n".into(), "Median".into(), "Mean rank".into()], rows: {
                let all: Vec<f64> = groups.iter().flatten().copied().collect();
                let rk = stats::ranks(&all);
                let mut off = 0;
                names.iter().zip(&groups).map(|(n, gv)| {
                    let mr = stats::mean(&rk[off..off + gv.len()]);
                    off += gv.len();
                    vec![n.clone(), gv.len().to_string(), stats::fmt(stats::quantile(&stats::sorted(gv), 0.5)), stats::fmt(mr)]
                }).collect()
            }, p_col: None });
            Ok(Out::ok(vec![], View::Report(rep), format!("p = {}", stats::fmt_p(r.p_value))))
        }
        "stats.decompose" => {
            let c = s_str(w, "value");
            if c.is_empty() {
                return Err(Out::idle("Choose the series."));
            }
            let x = e(series(&d, &c))?;
            let period = s_usize(w, "period").max(2);
            let dc = match s_str(w, "model").as_str() {
                "multiplicative" => e(stats::ts::decompose(&x, period, true))?,
                "stl" => e(stats::ts::stl(&x, period))?,
                _ => e(stats::ts::decompose(&x, period, false))?,
            };
            let nt = TableData {
                name: format!("{c} components"),
                columns: vec!["t".into(), "Observed".into(), "Trend".into(), "Seasonal".into(), "Residual".into()],
                types: vec![ColType::Integer, ColType::Number, ColType::Number, ColType::Number, ColType::Number],
                rows: (0..x.len()).map(|i| vec![Scalar::Num(i as f64 + 1.0), Scalar::Num(x[i]), num(dc.trend[i]), num(dc.seasonal[i]), num(dc.resid[i])]).collect(),
            };
            let s = format!("Trend strength {:.2} · seasonal strength {:.2}", dc.trend_strength, dc.seasonal_strength);
            Ok(Out::ok(vec![data_out(Dataset::new(nt))], View::Decomposition(dc), s))
        }
        "stats.acf" => {
            let c = s_str(w, "value");
            if c.is_empty() {
                return Err(Out::idle("Choose the series."));
            }
            let x = stats::ts::difference(&e(series(&d, &c))?, s_usize(w, "diff").min(2));
            let lags = s_usize(w, "lags").clamp(1, x.len().saturating_sub(2).max(1));
            let a = stats::ts::acf(&x, lags);
            let p = stats::ts::pacf(&x, lags);
            let (q, pv) = stats::ts::ljung_box(&x, lags.min(10), 0);
            Ok(Out::ok(vec![], View::Acf { acf: a, pacf: p, band: stats::ts::acf_band(x.len()), n: x.len() }, format!("Ljung–Box Q = {} (p {})", stats::fmt(q), stats::fmt_p(pv))))
        }
        "stats.arima" | "stats.ets" => {
            let c = s_str(w, "value");
            if c.is_empty() {
                return Err(Out::idle("Choose the series."));
            }
            let x = e(series(&d, &c))?;
            let h = s_usize(w, "horizon").clamp(1, 500);
            let level = s_f64(w, "level").clamp(0.5, 0.995);
            let tc = s_str(w, "time");
            let labels: Vec<String> = match t.column_index(if tc.is_empty() { "Period" } else { &tc }) {
                Some(i) => {
                    let ci = t.column_index(&c);
                    t.rows.iter().filter(|r| ci.and_then(|ci| r.get(ci)).and_then(Scalar::as_f64).is_some_and(f64::is_finite)).map(|r| r.get(i).map(Scalar::text).unwrap_or_default()).collect()
                }
                None => (1..=x.len()).map(|i| i.to_string()).collect(),
            };
            let (rep, fitted, fc) = if kind == "stats.arima" {
                let m = if s_bool(w, "auto") { e(stats::ts::auto_arima(&x, Some(s_usize(w, "d").min(2)), s_bool(w, "constant")))? } else { e(stats::ts::arima(&x, s_usize(w, "p"), s_usize(w, "d"), s_usize(w, "q"), s_bool(w, "constant")))? };
                (m.report(&c), m.fitted.clone(), m.forecast(h, level))
            } else {
                let kindm = match s_str(w, "method").as_str() {
                    "simple" => stats::ts::EtsKind::Simple,
                    "holt" => stats::ts::EtsKind::Holt,
                    "holt_winters_mul" => stats::ts::EtsKind::HoltWintersMultiplicative,
                    _ => stats::ts::EtsKind::HoltWintersAdditive,
                };
                let m = e(stats::ts::ets(&x, kindm, s_usize(w, "period").max(2)))?;
                (m.report(&c), m.fitted.clone(), m.forecast(h, level))
            };
            let n = x.len();
            let mut rows = vec![];
            for i in 0..n {
                rows.push(vec![Scalar::Text(labels.get(i).cloned().unwrap_or_else(|| (i + 1).to_string())), Scalar::Num(i as f64 + 1.0), Scalar::Num(x[i]), num(fitted.get(i).copied().unwrap_or(f64::NAN)), Scalar::Null, Scalar::Null, Scalar::Text("history".into())]);
            }
            for j in 0..fc.mean.len() {
                rows.push(vec![Scalar::Text(format!("+{}", j + 1)), Scalar::Num((n + j + 1) as f64), Scalar::Null, num(fc.mean[j]), num(fc.lower[j]), num(fc.upper[j]), Scalar::Text("forecast".into())]);
            }
            let nt = TableData { name: format!("{c} forecast"), columns: vec!["Period".into(), "t".into(), c.clone(), "Forecast".into(), "Lower".into(), "Upper".into(), "Kind".into()], types: vec![ColType::Text, ColType::Integer, ColType::Number, ColType::Number, ColType::Number, ColType::Number, ColType::Text], rows };
            let s = format!("{} · next: {}", rep.title, fc.mean.first().map(|v| stats::fmt(*v)).unwrap_or_default());
            Ok(Out::ok(vec![data_out(Dataset::new(nt))], View::Forecast { report: rep, history: x, fitted, forecast: fc, labels }, s))
        }
        "stats.distfit" => {
            let c = s_str(w, "column");
            if c.is_empty() {
                return Err(Out::idle("Choose the variable."));
            }
            let v = e(series(&d, &c))?;
            let fits = stats::dist::fit_all(&v);
            let rep = stats::dist::report(&fits, &c);
            let s = fits.first().map(|f| format!("Best: {}", f.dist.name())).unwrap_or_default();
            Ok(Out::ok(vec![], View::Distribution { report: rep, values: v, fits }, s))
        }
        // -------------------------------------------------------------- deploy
        "deploy.score" | "deploy.drift" => {
            let name = s_str(w, "model");
            if name.is_empty() {
                return Err(Out::idle("Choose a registered model."));
            }
            let ver = s_str(w, "version");
            let m = if ver.trim().is_empty() { registry::serving_version(ctx.registry, &name) } else { ver.trim().trim_start_matches('v').parse::<u32>().ok().and_then(|v| ctx.registry.iter().find(|m| m.name == name && m.version == v)) };
            let m = m.ok_or_else(|| Out::err(format!("No registered model `{name}`{}.", if ver.is_empty() { String::new() } else { format!(" version {ver}") })))?;
            if kind == "deploy.score" {
                let col = format!("{} (v{})", m.target, m.version);
                let col = format!("Predicted {col}");
                let before = t.columns.len();
                let nt = registry::score_table(&m.model, t, &col, s_bool(w, "probs"));
                let mut nd = d.with(nt);
                nd.metas.extend(nd.table.columns[before..].iter().cloned());
                let s = format!("{} rows scored with {} v{}", fmt_int(t.rows.len()), m.name, m.version);
                return Ok(Out::ok(vec![data_out(nd)], View::None, s));
            }
            let rows = registry::drift(m, t);
            let worst = rows.iter().map(|r| r.psi).fold(0.0, f64::max);
            let drifted = rows.iter().filter(|r| r.psi >= 0.25).count();
            let nt = TableData {
                name: "Drift report".into(),
                columns: ["Feature", "PSI", "KS", "KS p-value", "Train mean", "New mean", "Status"].iter().map(|s| s.to_string()).collect(),
                types: vec![ColType::Text, ColType::Number, ColType::Number, ColType::Number, ColType::Number, ColType::Number, ColType::Text],
                rows: rows.iter().map(|r| vec![Scalar::Text(r.feature.clone()), num(r.psi), num(r.ks), num(r.ks_p), num(r.train_mean), num(r.new_mean), Scalar::Text(r.status.clone())]).collect(),
            };
            let mut out = Out::ok(vec![data_out(Dataset::new(nt))], View::Drift(rows), format!("Max PSI {worst:.3} · {drifted} drifting"));
            if drifted > 0 {
                out.state = State::Warning;
                out.message = format!("{drifted} feature(s) show significant drift (PSI ≥ 0.25).");
            }
            Ok(out)
        }
        "deploy.save" => {
            let s = format!("{} → table \"{}\"", d.summary(), s_str(w, "name"));
            Ok(Out::ok(vec![], View::None, s))
        }
        other => Err(Out::err(format!("Unknown widget `{other}`."))),
    }
}

fn num(v: f64) -> Scalar {
    if v.is_finite() { Scalar::Num(v) } else { Scalar::Null }
}

/// Metrics of a fitted model for the registry: cross-validated scores when evaluation results
/// for it are available, else training-set scores.
pub fn model_metrics(m: &Learned, train: &Dataset, eval_results: Option<&EvalResults>) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    if let Some(r) = eval_results
        && let Some(i) = r.learners.iter().position(|l| *l == m.name())
        && let Some(s) = eval::scores(r).get(i)
    {
        for (k, v) in &s.values {
            if v.is_finite() {
                out.insert(format!("cv_{k}"), *v);
            }
        }
    }
    if let Ok(res) = eval::evaluate(&[m.spec.clone()], &train.table, &train.features(), &m.target, &Sampling::TrainOnTest, 1, None)
        && let Some(s) = eval::scores(&res).first()
    {
        for (k, v) in &s.values {
            if v.is_finite() {
                out.insert(format!("train_{k}"), *v);
            }
        }
    }
    out
}

/// Helper for UI: JSON value of a setting as display text.
pub fn display(v: &J) -> String {
    match v {
        J::String(s) => s.clone(),
        J::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::studio::learn::tests_support::sample;

    #[test]
    fn workflow_runs_end_to_end() {
        let mut tables = BTreeMap::new();
        tables.insert("S".to_string(), Rc::new(sample(200)));
        let ctx = Ctx { tables: &tables, generation: 1, registry: &[], base_dir: None };
        let mut wf = StudioWorkflow { name: "t".into(), ..Default::default() };
        let src = wf.add("data.table", 0.0, 0.0);
        wf.widget_mut(src).unwrap().settings.insert("table".into(), J::from("S"));
        let sel = wf.add("data.select_columns", 0.0, 0.0);
        wf.widget_mut(sel).unwrap().settings.insert("target".into(), J::from("label"));
        wf.widget_mut(sel).unwrap().settings.insert("ignore".into(), serde_json::json!(["y"]));
        let rf = wf.add("model.forest", 0.0, 0.0);
        let lr = wf.add("model.logistic", 0.0, 0.0);
        let ts = wf.add("eval.test_score", 0.0, 0.0);
        let roc = wf.add("eval.roc", 0.0, 0.0);
        let pr = wf.add("eval.predictions", 0.0, 0.0);
        let km = wf.add("unsup.kmeans", 0.0, 0.0);
        let ols = wf.add("stats.ols", 0.0, 0.0);
        wf.widget_mut(ols).unwrap().settings.insert("target".into(), J::from("y"));
        wf.widget_mut(ols).unwrap().settings.insert("predictors".into(), serde_json::json!(["a", "b", "c"]));
        wf.connect(src, 0, sel, 0).unwrap();
        wf.connect(sel, 0, rf, 0).unwrap();
        wf.connect(sel, 0, lr, 0).unwrap();
        wf.connect(sel, 0, ts, 0).unwrap();
        wf.connect(rf, 0, ts, 1).unwrap();
        wf.connect(lr, 0, ts, 1).unwrap();
        wf.connect(ts, 0, roc, 0).unwrap();
        wf.connect(sel, 0, pr, 0).unwrap();
        wf.connect(rf, 0, pr, 1).unwrap();
        wf.connect(sel, 0, km, 0).unwrap();
        wf.connect(src, 0, ols, 0).unwrap();
        let mut ex = Executor::default();
        ex.run(&ctx, &wf);
        for w in &wf.widgets {
            let o = ex.get(w.id).unwrap();
            assert_eq!(o.state, State::Ok, "{}: {}", w.label, o.message);
        }
        let View::Scores { scores, .. } = &ex.get(ts).unwrap().view else { panic!() };
        assert_eq!(scores.len(), 2);
        assert!(scores.iter().all(|s| s.get("AUC") > 0.8));
        // Cached: a second run reuses outputs.
        let before = ex.get(ts).unwrap();
        ex.run(&ctx, &wf);
        assert!(Rc::ptr_eq(&before, &ex.get(ts).unwrap()));
        // Changing a setting re-runs downstream.
        wf.widget_mut(rf).unwrap().settings.insert("trees".into(), J::from(5));
        ex.run(&ctx, &wf);
        assert!(!Rc::ptr_eq(&before, &ex.get(ts).unwrap()));
    }
}
