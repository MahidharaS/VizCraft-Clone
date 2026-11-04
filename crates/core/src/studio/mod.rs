//! Data Science Studio: an Orange-style visual programming workbench for machine learning and
//! statistical modelling, with MLOps (model registry, deployment, monitoring).
//!
//! A [`StudioWorkflow`] is a graph of widgets (data sources, transforms, learners, evaluators,
//! visualisations, statistical tests, deployment steps) joined by typed channels (Data, Model,
//! Evaluation results). The [`catalog`] describes every widget; [`exec::Executor`] runs the graph
//! with per-widget caching; [`learn`], [`eval`], [`unsup`], [`rank`] hold the algorithms and
//! [`registry`] the model registry records, drift monitoring and model export.

pub mod eval;
pub mod examples;
pub mod exec;
pub mod expr;
pub mod learn;
pub mod rank;
pub mod registry;
pub mod unsup;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value as J, json};

pub use registry::{RegisteredModel, Stage};

// =============================================================================================
// Document types (stored in `Project::studio`)
// =============================================================================================

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StudioWorkflow {
    pub name: String,
    pub widgets: Vec<StudioWidget>,
    pub links: Vec<StudioLink>,
    pub description: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StudioWidget {
    pub id: u64,
    /// Widget type id (see [`catalog`]), e.g. "model.forest".
    pub kind: String,
    pub label: String,
    pub x: f32,
    pub y: f32,
    pub settings: BTreeMap<String, J>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StudioLink {
    pub from: u64,
    pub from_port: usize,
    pub to: u64,
    pub to_port: usize,
}

impl StudioWorkflow {
    pub fn widget(&self, id: u64) -> Option<&StudioWidget> {
        self.widgets.iter().find(|w| w.id == id)
    }
    pub fn widget_mut(&mut self, id: u64) -> Option<&mut StudioWidget> {
        self.widgets.iter_mut().find(|w| w.id == id)
    }
    pub fn next_id(&self) -> u64 {
        self.widgets.iter().map(|w| w.id).max().unwrap_or(0) + 1
    }
    /// Adds a widget with default settings; returns its id.
    pub fn add(&mut self, kind: &str, x: f32, y: f32) -> u64 {
        let id = self.next_id();
        let t = widget_type(kind);
        let mut settings = BTreeMap::new();
        if let Some(t) = &t {
            for p in &t.params {
                settings.insert(p.key.to_string(), p.default.clone());
            }
        }
        let label = t.map(|t| t.name.to_string()).unwrap_or_else(|| kind.to_string());
        // Unique label.
        let mut l = label.clone();
        let mut i = 2;
        while self.widgets.iter().any(|w| w.label == l) {
            l = format!("{label} ({i})");
            i += 1;
        }
        self.widgets.push(StudioWidget { id, kind: kind.into(), label: l, x, y, settings });
        id
    }
    /// Would linking `from → to` create a cycle?
    pub fn creates_cycle(&self, from: u64, to: u64) -> bool {
        if from == to {
            return true;
        }
        let mut stack = vec![to];
        let mut seen = std::collections::HashSet::new();
        while let Some(n) = stack.pop() {
            if n == from {
                return true;
            }
            if seen.insert(n) {
                stack.extend(self.links.iter().filter(|l| l.from == n).map(|l| l.to));
            }
        }
        false
    }
    /// Connects two ports (replacing an existing link into a single-input port).
    pub fn connect(&mut self, from: u64, from_port: usize, to: u64, to_port: usize) -> Result<(), String> {
        let (Some(a), Some(b)) = (self.widget(from), self.widget(to)) else { return Err("Unknown widget.".into()) };
        let (Some(ta), Some(tb)) = (widget_type(&a.kind), widget_type(&b.kind)) else { return Err("Unknown widget type.".into()) };
        let (Some(op), Some(ip)) = (ta.outputs.get(from_port), tb.inputs.get(to_port)) else { return Err("Unknown port.".into()) };
        if !op.chan.fits(ip.chan) {
            return Err(format!("{} can't connect to {} ({} → {}).", op.name, ip.name, op.chan.label(), ip.chan.label()));
        }
        if self.creates_cycle(from, to) {
            return Err("That link would create a loop.".into());
        }
        if !ip.multiple {
            self.links.retain(|l| !(l.to == to && l.to_port == to_port));
        }
        let link = StudioLink { from, from_port, to, to_port };
        if !self.links.contains(&link) {
            self.links.push(link);
        }
        Ok(())
    }
    /// Links into `id` by input port.
    pub fn inputs_of(&self, id: u64) -> Vec<&StudioLink> {
        self.links.iter().filter(|l| l.to == id).collect()
    }
    pub fn remove(&mut self, id: u64) {
        self.widgets.retain(|w| w.id != id);
        self.links.retain(|l| l.from != id && l.to != id);
    }
}

// =============================================================================================
// Channels, ports, parameters
// =============================================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Chan {
    Data,
    Model,
    Eval,
}

impl Chan {
    pub fn label(self) -> &'static str {
        match self {
            Chan::Data => "Data",
            Chan::Model => "Model",
            Chan::Eval => "Evaluation Results",
        }
    }
    pub fn fits(self, input: Chan) -> bool {
        self == input
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PortDef {
    pub name: &'static str,
    pub chan: Chan,
    /// Accepts several links (e.g. the learners of Test & Score).
    pub multiple: bool,
    pub optional: bool,
}

const fn port(name: &'static str, chan: Chan) -> PortDef {
    PortDef { name, chan, multiple: false, optional: false }
}
const fn opt(name: &'static str, chan: Chan) -> PortDef {
    PortDef { name, chan, multiple: false, optional: true }
}
const fn many(name: &'static str, chan: Chan) -> PortDef {
    PortDef { name, chan, multiple: true, optional: false }
}

/// Which columns a column parameter offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColKind {
    Any,
    Numeric,
    Categorical,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ParamKind {
    /// A model table name.
    Table,
    /// A file path.
    File,
    /// One column of the data on input `port`; `optional` allows "(none)".
    Column { kind: ColKind, port: usize, optional: bool },
    /// Several columns (empty = all suitable).
    Columns { kind: ColKind, port: usize },
    Choice(&'static [(&'static str, &'static str)]),
    Int(i64, i64),
    Float(f64, f64),
    Bool,
    Text,
    /// Multi-line text (expressions).
    Code,
    /// A learner of the incoming evaluation results (index).
    Learner,
    /// A target class of the incoming evaluation results (index).
    Class,
    /// A registered model name.
    Registered,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParamDef {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: ParamKind,
    pub default: J,
    pub help: &'static str,
}

fn p(key: &'static str, label: &'static str, kind: ParamKind, default: J) -> ParamDef {
    ParamDef { key, label, kind, default, help: "" }
}
fn col(key: &'static str, label: &'static str, kind: ColKind) -> ParamDef {
    p(key, label, ParamKind::Column { kind, port: 0, optional: false }, J::from(""))
}
fn ocol(key: &'static str, label: &'static str, kind: ColKind) -> ParamDef {
    p(key, label, ParamKind::Column { kind, port: 0, optional: true }, J::from(""))
}
fn cols(key: &'static str, label: &'static str, kind: ColKind) -> ParamDef {
    p(key, label, ParamKind::Columns { kind, port: 0 }, json!([]))
}
fn int(key: &'static str, label: &'static str, lo: i64, hi: i64, d: i64) -> ParamDef {
    p(key, label, ParamKind::Int(lo, hi), J::from(d))
}
fn float(key: &'static str, label: &'static str, lo: f64, hi: f64, d: f64) -> ParamDef {
    p(key, label, ParamKind::Float(lo, hi), J::from(d))
}
fn choice(key: &'static str, label: &'static str, opts: &'static [(&'static str, &'static str)]) -> ParamDef {
    p(key, label, ParamKind::Choice(opts), J::from(opts.first().map(|o| o.0).unwrap_or("")))
}
fn boolean(key: &'static str, label: &'static str, d: bool) -> ParamDef {
    p(key, label, ParamKind::Bool, J::from(d))
}
fn text(key: &'static str, label: &'static str, d: &str) -> ParamDef {
    p(key, label, ParamKind::Text, J::from(d))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Data,
    Transform,
    Model,
    Evaluate,
    Visualize,
    Unsupervised,
    Statistics,
    Deploy,
}

impl Category {
    pub const ALL: [Category; 8] = [Category::Data, Category::Transform, Category::Model, Category::Evaluate, Category::Visualize, Category::Unsupervised, Category::Statistics, Category::Deploy];
    pub fn label(self) -> &'static str {
        match self {
            Category::Data => "Data",
            Category::Transform => "Transform",
            Category::Model => "Model",
            Category::Evaluate => "Evaluate",
            Category::Visualize => "Visualize",
            Category::Unsupervised => "Unsupervised",
            Category::Statistics => "Statistics",
            Category::Deploy => "Deploy",
        }
    }
    /// Category colour (RGB), used for the round widget nodes.
    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            Category::Data => (0xF2, 0x9E, 0x38),
            Category::Transform => (0xE5, 0x6B, 0x5D),
            Category::Model => (0xE0, 0x5E, 0x9A),
            Category::Evaluate => (0x9B, 0x6B, 0xD8),
            Category::Visualize => (0xF0, 0xBE, 0x3C),
            Category::Unsupervised => (0x4F, 0xA3, 0xD9),
            Category::Statistics => (0x2E, 0xB0, 0x8E),
            Category::Deploy => (0x5B, 0x6E, 0xE1),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WidgetType {
    pub id: &'static str,
    pub name: &'static str,
    pub category: Category,
    pub description: &'static str,
    pub inputs: Vec<PortDef>,
    pub outputs: Vec<PortDef>,
    pub params: Vec<ParamDef>,
}

fn w(id: &'static str, name: &'static str, category: Category, description: &'static str, inputs: Vec<PortDef>, outputs: Vec<PortDef>, params: Vec<ParamDef>) -> WidgetType {
    WidgetType { id, name, category, description, inputs, outputs, params }
}

const SEED_MAX: i64 = 1_000_000;

fn learner(id: &'static str, name: &'static str, desc: &'static str, mut params: Vec<ParamDef>) -> WidgetType {
    params.insert(0, text("name", "Name", ""));
    w(id, name, Category::Model, desc, vec![opt("Data", Chan::Data)], vec![port("Model", Chan::Model)], params)
}

/// Every widget type.
pub fn catalog() -> Vec<WidgetType> {
    use Category::{Deploy, Evaluate, Statistics, Transform, Unsupervised, Visualize};
    use Chan::*;
    use ColKind::*;
    let d_in = || vec![port("Data", Data)];
    let d_out = || vec![port("Data", Data)];
    vec![
        // ---------------------------------------------------------------- Data
        w("data.table", "Model Table", Category::Data, "A table of the project model (Power Query output, data sheet, flow output).", vec![], d_out(), vec![p("table", "Table", ParamKind::Table, J::from(""))]),
        w("data.csv", "CSV File", Category::Data, "Reads a delimited text file.", vec![], d_out(), vec![p("path", "File", ParamKind::File, J::from("")), choice("delimiter", "Delimiter", &[("auto", "Detect"), (",", "Comma"), (";", "Semicolon"), ("\t", "Tab"), ("|", "Pipe")]), boolean("header", "First row is the header", true)]),
        w("data.datatable", "Data Table", Category::Data, "Shows the data in a spreadsheet-like grid.", d_in(), d_out(), vec![]),
        w("data.select_columns", "Select Columns", Category::Data, "Sets the target variable and ignores or keeps columns as meta attributes.", d_in(), d_out(), vec![ocol("target", "Target variable", Any), cols("ignore", "Ignored columns", Any), cols("metas", "Meta attributes (kept, not used)", Any)]),
        w(
            "data.select_rows",
            "Select Rows",
            Category::Data,
            "Keeps the rows matching a condition.",
            d_in(),
            vec![port("Matching Data", Data), port("Unmatched Data", Data)],
            vec![ocol("column", "Column", Any), choice("op", "Condition", &[("==", "equals"), ("!=", "is not"), ("<", "is below"), ("<=", "is at most"), (">", "is above"), (">=", "is at least"), ("contains", "contains"), ("missing", "is missing"), ("present", "is not missing")]), text("value", "Value", "")],
        ),
        w(
            "data.merge",
            "Merge Data",
            Category::Data,
            "Joins two tables on key columns (left / inner / full outer join).",
            vec![port("Data", Data), port("Extra Data", Data)],
            d_out(),
            vec![col("left_key", "Key in Data", Any), p("right_key", "Key in Extra Data", ParamKind::Column { kind: Any, port: 1, optional: false }, J::from("")), choice("how", "Join", &[("left", "Append columns (left join)"), ("inner", "Find matching pairs (inner)"), ("full", "Concatenate (full outer)")])],
        ),
        w(
            "data.sample",
            "Data Sampler",
            Category::Data,
            "Draws a random sample (fixed proportion, fixed size or bootstrap); the rest goes to Remaining Data.",
            d_in(),
            vec![port("Data Sample", Data), port("Remaining Data", Data)],
            vec![choice("mode", "Sampling", &[("proportion", "Fixed proportion"), ("size", "Fixed sample size"), ("bootstrap", "Bootstrap")]), float("proportion", "Proportion", 0.01, 0.99, 0.7), int("size", "Sample size", 1, 10_000_000, 100), boolean("stratify", "Stratify by the target", true), int("seed", "Random seed", 0, SEED_MAX, 42)],
        ),
        w("data.feature_stats", "Feature Statistics", Category::Data, "Distribution, centre, dispersion and missing values of every column.", d_in(), d_out(), vec![]),
        // ---------------------------------------------------------------- Transform
        w(
            "tf.preprocess",
            "Preprocess",
            Transform,
            "Imputation, scaling and removal of constant features.",
            d_in(),
            d_out(),
            vec![choice("impute", "Missing values", &[("mean", "Replace with mean / mode"), ("median", "Replace with median"), ("drop", "Remove rows"), ("none", "Keep")]), choice("scale", "Normalize", &[("none", "No scaling"), ("standardize", "Standardize (μ = 0, σ = 1)"), ("minmax", "Normalize to [0, 1]")]), boolean("drop_constant", "Remove constant features", true), cols("columns", "Numeric columns (empty = all)", Numeric)],
        ),
        w("tf.impute", "Impute", Transform, "Replaces missing values.", d_in(), d_out(), vec![choice("method", "Method", &[("mean", "Average / most frequent"), ("median", "Median"), ("constant", "Fixed value"), ("drop", "Remove rows with missing values")]), float("value", "Fixed value", -1e12, 1e12, 0.0), cols("columns", "Columns (empty = all)", Any)]),
        w("tf.constructor", "Feature Constructor", Transform, "Adds columns computed from expressions (one `name = expression` per line).", d_in(), d_out(), vec![p("code", "Features", ParamKind::Code, J::from("Revenue = Quantity * [Unit Price] * (1 - Discount)"))]),
        w("tf.polynomial", "Polynomial Features", Transform, "Adds powers and pairwise products of numeric columns.", d_in(), d_out(), vec![cols("columns", "Columns", Numeric), int("degree", "Degree", 2, 3, 2), boolean("interactions", "Interaction terms", true)]),
        w("tf.discretize", "Discretize", Transform, "Turns numeric columns into intervals.", d_in(), d_out(), vec![cols("columns", "Columns (empty = all numeric)", Numeric), int("bins", "Intervals", 2, 20, 4), choice("method", "Method", &[("frequency", "Equal frequency"), ("width", "Equal width")])]),
        w(
            "tf.timeagg",
            "Time Aggregate",
            Transform,
            "Aggregates a value by calendar period (day, week, month, quarter, year): a time series.",
            d_in(),
            d_out(),
            vec![col("date", "Date column", Any), ocol("value", "Value", Numeric), choice("period", "Period", &[("month", "Month"), ("week", "Week"), ("day", "Day"), ("quarter", "Quarter"), ("year", "Year")]), choice("agg", "Aggregate", &[("sum", "Sum"), ("mean", "Average"), ("count", "Count")])],
        ),
        // ---------------------------------------------------------------- Model
        learner("model.knn", "kNN", "k-nearest neighbours (standardized features).", vec![int("k", "Neighbours", 1, 200, 5), choice("weights", "Weights", &[("uniform", "Uniform"), ("distance", "By distance")])]),
        learner("model.tree", "Tree", "A CART decision tree.", vec![int("max_depth", "Max depth", 1, 30, 8), int("min_samples_leaf", "Min instances in leaves", 1, 500, 2), int("min_samples_split", "Don't split subsets smaller than", 2, 1000, 4)]),
        learner("model.forest", "Random Forest", "Bagged trees with random feature subsets.", vec![int("trees", "Trees", 1, 500, 50), int("max_depth", "Max depth", 1, 30, 10), choice("max_features", "Features per split", &[("sqrt", "√ features"), ("log2", "log₂ features"), ("third", "⅓ of features"), ("all", "All")]), int("min_samples_leaf", "Min instances in leaves", 1, 500, 2), int("seed", "Random seed", 0, SEED_MAX, 42)]),
        learner("model.svm", "SVM", "Linear support vector machine (hinge loss; ε-insensitive for regression).", vec![float("C", "Cost (C)", 0.001, 1000.0, 1.0), float("epsilon", "Regression ε", 0.0, 10.0, 0.1), int("iterations", "Epochs", 1, 500, 40), int("seed", "Random seed", 0, SEED_MAX, 42)]),
        learner("model.linear", "Linear Regression", "OLS, ridge, lasso or elastic net.", vec![choice("penalty", "Regularization", &[("none", "None (OLS)"), ("ridge", "Ridge (L2)"), ("lasso", "Lasso (L1)"), ("elastic", "Elastic net")]), float("alpha", "Strength (α)", 0.0, 1000.0, 1.0), float("l1_ratio", "L1 ratio (elastic net)", 0.0, 1.0, 0.5)]),
        learner("model.logistic", "Logistic Regression", "Multinomial logistic regression with L2 penalty.", vec![float("C", "Strength (C, inverse)", 0.001, 1000.0, 1.0), int("iterations", "Iterations", 10, 5000, 300)]),
        learner("model.naive_bayes", "Naive Bayes", "Gaussian naive Bayes.", vec![]),
        learner("model.mlp", "Neural Network", "Multilayer perceptron (1–2 hidden layers, Adam).", vec![text("hidden", "Neurons per hidden layer", "32"), choice("activation", "Activation", &[("relu", "ReLU"), ("tanh", "tanh"), ("logistic", "Logistic")]), float("alpha", "Regularization (α)", 0.0, 10.0, 0.0001), float("learning_rate", "Learning rate", 0.00001, 1.0, 0.01), int("iterations", "Epochs", 1, 2000, 200), int("seed", "Random seed", 0, SEED_MAX, 42)]),
        learner("model.gbm", "Gradient Boosting", "Boosted shallow trees (squared loss / log-loss one-vs-rest).", vec![int("trees", "Trees", 1, 1000, 100), float("learning_rate", "Learning rate", 0.001, 1.0, 0.1), int("max_depth", "Max depth", 1, 12, 3), float("subsample", "Fraction of training instances", 0.1, 1.0, 1.0), int("seed", "Random seed", 0, SEED_MAX, 42)]),
        learner("model.adaboost", "AdaBoost", "Adaptive boosting of shallow trees (SAMME; AdaBoost.R2 for regression).", vec![int("trees", "Estimators", 1, 500, 50), float("learning_rate", "Learning rate", 0.01, 2.0, 1.0), int("max_depth", "Base tree depth", 1, 10, 1), int("seed", "Random seed", 0, SEED_MAX, 42)]),
        // ---------------------------------------------------------------- Evaluate
        w(
            "eval.test_score",
            "Test & Score",
            Evaluate,
            "Cross-validates the connected learners and reports scores.",
            vec![port("Data", Data), many("Learner", Model), opt("Test Data", Data)],
            vec![port("Evaluation Results", Eval)],
            vec![choice("sampling", "Sampling", &[("stratified", "Stratified k-fold"), ("kfold", "k-fold"), ("random", "Random sampling"), ("loo", "Leave one out"), ("train", "Test on train data")]), int("folds", "Folds", 2, 20, 5), int("repeats", "Repeats (random sampling)", 1, 50, 10), float("train_size", "Training set size", 0.05, 0.95, 0.75), int("seed", "Random seed", 0, SEED_MAX, 42)],
        ),
        w("eval.predictions", "Predictions", Evaluate, "Applies models to data: predicted values and class probabilities.", vec![port("Data", Data), many("Predictors", Model)], vec![port("Predictions", Data)], vec![boolean("probs", "Show probabilities", true)]),
        w("eval.confusion", "Confusion Matrix", Evaluate, "Actual vs predicted classes.", vec![port("Evaluation Results", Eval)], vec![], vec![p("learner", "Learner", ParamKind::Learner, J::from(0)), choice("show", "Show", &[("count", "Number of instances"), ("row", "Proportion of actual"), ("col", "Proportion of predicted")])]),
        w("eval.roc", "ROC Analysis", Evaluate, "Receiver operating characteristic curves with AUC.", vec![port("Evaluation Results", Eval)], vec![], vec![p("class", "Target class", ParamKind::Class, J::from(1))]),
        w("eval.lift", "Lift Curve", Evaluate, "Cumulative gains and lift.", vec![port("Evaluation Results", Eval)], vec![], vec![p("class", "Target class", ParamKind::Class, J::from(1)), choice("curve", "Curve", &[("lift", "Lift"), ("gain", "Cumulative gain")])]),
        w("eval.calibration", "Calibration Plot", Evaluate, "Predicted probability vs observed frequency.", vec![port("Evaluation Results", Eval)], vec![], vec![p("class", "Target class", ParamKind::Class, J::from(1)), int("bins", "Bins", 3, 30, 10)]),
        w("eval.rank", "Rank", Evaluate, "Scores features by their relation to the target (information gain, Gini, χ², ANOVA, regression F).", d_in(), vec![port("Reduced Data", Data)], vec![int("top", "Keep the top features (0 = all)", 0, 500, 0)]),
        w("eval.importance", "Feature Importance", Evaluate, "Permutation importance of each feature for a fitted model.", vec![port("Data", Data), port("Model", Model)], vec![], vec![int("repeats", "Repeats", 1, 20, 5), int("seed", "Random seed", 0, SEED_MAX, 42)]),
        w("eval.explain", "Explain Prediction", Evaluate, "Contribution of each feature to one prediction.", vec![port("Data", Data), port("Model", Model)], vec![], vec![int("row", "Row", 0, 10_000_000, 0), p("class", "Target class", ParamKind::Class, J::from(1))]),
        // ---------------------------------------------------------------- Visualize
        w("viz.scatter", "Scatter Plot", Visualize, "Two numeric columns, coloured / sized / shaped by others, with a regression line.", d_in(), vec![], vec![col("x", "Axis x", Numeric), col("y", "Axis y", Numeric), ocol("color", "Color", Any), ocol("size", "Size", Numeric), ocol("shape", "Shape", Categorical), boolean("regression", "Regression line", true), float("jitter", "Jittering", 0.0, 0.2, 0.0)]),
        w("viz.distributions", "Distributions", Visualize, "Histogram of a column, split by a categorical column.", d_in(), vec![], vec![col("column", "Variable", Any), ocol("split", "Split by", Categorical), int("bins", "Bins", 2, 100, 20), boolean("stacked", "Stack bars", true), boolean("density", "Show density curve", true)]),
        w("viz.box", "Box Plot", Visualize, "Box plots of a numeric column by group, with an ANOVA test.", d_in(), vec![], vec![col("column", "Variable", Numeric), ocol("group", "Subgroups", Categorical), boolean("test", "Show the test", true)]),
        w("viz.heatmap", "Heat Map", Visualize, "Standardized values of numeric columns as colours; rows can be clustered.", d_in(), vec![], vec![cols("columns", "Columns (empty = all numeric)", Numeric), boolean("cluster", "Cluster rows", true), int("max_rows", "Rows shown", 10, 2000, 200)]),
        w("viz.correlations", "Correlations", Visualize, "Pairwise correlations ranked by strength.", d_in(), vec![], vec![choice("method", "Method", &[("pearson", "Pearson"), ("spearman", "Spearman")]), cols("columns", "Columns (empty = all numeric)", Numeric)]),
        w("viz.sieve", "Sieve Diagram", Visualize, "Observed vs expected frequencies of two categorical columns.", d_in(), vec![], vec![col("x", "Columns", Categorical), col("y", "Rows", Categorical)]),
        w("viz.mosaic", "Mosaic Display", Visualize, "Proportions of one categorical column within another.", d_in(), vec![], vec![col("x", "Split by", Categorical), col("y", "Colour by", Categorical)]),
        w("viz.line", "Line Chart", Visualize, "Numeric columns over a column (time series).", d_in(), vec![], vec![col("x", "Axis x", Any), cols("y", "Series", Numeric)]),
        // ---------------------------------------------------------------- Unsupervised
        w("unsup.kmeans", "k-Means", Unsupervised, "k-means clustering (k-means++); can pick k by silhouette.", d_in(), d_out(), vec![int("k", "Number of clusters", 2, 30, 3), boolean("auto", "Choose k by silhouette (2–8)", false), cols("columns", "Columns (empty = all numeric)", Numeric), int("seed", "Random seed", 0, SEED_MAX, 42)]),
        w("unsup.hclust", "Hierarchical Clustering", Unsupervised, "Agglomerative clustering with a dendrogram.", d_in(), d_out(), vec![choice("linkage", "Linkage", &[("ward", "Ward"), ("average", "Average"), ("complete", "Complete"), ("single", "Single")]), int("k", "Clusters", 1, 50, 3), ocol("label", "Leaf labels", Any), cols("columns", "Columns (empty = all numeric)", Numeric), int("max_rows", "Max rows", 10, 1500, 300)]),
        w("unsup.dbscan", "DBSCAN", Unsupervised, "Density-based clustering on standardized data.", d_in(), d_out(), vec![float("eps", "Neighbourhood distance", 0.01, 100.0, 0.5), int("min_pts", "Core point neighbours", 1, 100, 5), cols("columns", "Columns (empty = all numeric)", Numeric)]),
        w("unsup.pca", "PCA", Unsupervised, "Principal component analysis: scree plot, loadings and projection.", d_in(), d_out(), vec![int("components", "Components", 1, 50, 2), boolean("standardize", "Normalize variables", true), cols("columns", "Columns (empty = all numeric)", Numeric)]),
        w("unsup.mds", "MDS", Unsupervised, "Classical multidimensional scaling to 2-D.", d_in(), d_out(), vec![cols("columns", "Columns (empty = all numeric)", Numeric), int("max_rows", "Max rows", 10, 2000, 500), int("seed", "Random seed", 0, SEED_MAX, 42)]),
        w("unsup.tsne", "t-SNE", Unsupervised, "t-distributed stochastic neighbour embedding to 2-D.", d_in(), d_out(), vec![float("perplexity", "Perplexity", 2.0, 100.0, 30.0), int("iterations", "Iterations", 100, 2000, 500), cols("columns", "Columns (empty = all numeric)", Numeric), int("max_rows", "Max rows", 10, 2000, 500), int("seed", "Random seed", 0, SEED_MAX, 42)]),
        w("unsup.silhouette", "Silhouette Plot", Unsupervised, "How well each row fits its cluster.", d_in(), vec![], vec![col("cluster", "Cluster column", Any), cols("columns", "Columns (empty = all numeric)", Numeric), int("max_rows", "Max rows", 10, 3000, 1000)]),
        // ---------------------------------------------------------------- Statistics
        w("stats.describe", "Descriptive Statistics", Statistics, "Mean, standard deviation, quantiles, skewness, kurtosis and confidence intervals, optionally by group.", d_in(), d_out(), vec![cols("columns", "Columns (empty = all numeric)", Numeric), ocol("group", "Group by", Categorical)]),
        w("stats.ols", "Linear Regression (OLS)", Statistics, "Ordinary least squares with full inference (statsmodels-style summary).", d_in(), vec![port("Residuals", Data)], vec![col("target", "Dependent variable", Numeric), cols("predictors", "Predictors", Any)]),
        w("stats.logit", "Logistic Regression (Logit)", Statistics, "Binary logistic regression with p-values and odds ratios.", d_in(), vec![], vec![col("target", "Dependent variable (binary)", Any), text("event", "Event class (empty = last)", ""), cols("predictors", "Predictors", Any)]),
        w("stats.glm", "Poisson Regression", Statistics, "GLM for counts (Poisson family, log link).", d_in(), vec![], vec![col("target", "Count variable", Numeric), cols("predictors", "Predictors", Any)]),
        w(
            "stats.ttest",
            "t-Test",
            Statistics,
            "One-sample, two-sample (Student or Welch) and paired t-tests.",
            d_in(),
            vec![],
            vec![choice("test", "Test", &[("welch", "Two samples (Welch)"), ("student", "Two samples (equal variances)"), ("one", "One sample"), ("paired", "Paired samples")]), col("column", "Variable", Numeric), ocol("group", "Groups (two-sample)", Categorical), text("a", "Group 1 (empty = first)", ""), text("b", "Group 2 (empty = second)", ""), ocol("column2", "Second variable (paired)", Numeric), float("mu", "Hypothesised mean", -1e12, 1e12, 0.0)],
        ),
        w("stats.anova", "ANOVA", Statistics, "One-way or two-way analysis of variance with Tukey HSD.", d_in(), vec![], vec![col("value", "Variable", Numeric), col("factor", "Factor", Categorical), ocol("factor2", "Second factor (two-way)", Categorical), boolean("tukey", "Tukey HSD post-hoc", true)]),
        w("stats.chisq", "Chi-Square Test", Statistics, "Test of independence of two categorical columns.", d_in(), vec![], vec![col("a", "Rows", Categorical), col("b", "Columns", Categorical)]),
        w("stats.correlation", "Correlation Test", Statistics, "Pearson, Spearman or Kendall correlation with a p-value.", d_in(), vec![], vec![col("x", "Variable 1", Numeric), col("y", "Variable 2", Numeric), choice("method", "Method", &[("pearson", "Pearson"), ("spearman", "Spearman"), ("kendall", "Kendall τ-b")])]),
        w("stats.normality", "Normality Test", Statistics, "Shapiro–Wilk and Jarque–Bera tests with a Q–Q plot.", d_in(), vec![], vec![col("column", "Variable", Numeric)]),
        w("stats.nonparam", "Rank Tests", Statistics, "Mann–Whitney U (two groups) or Kruskal–Wallis H (several groups).", d_in(), vec![], vec![col("value", "Variable", Numeric), col("group", "Groups", Categorical)]),
        w("stats.decompose", "Seasonal Decomposition", Statistics, "Trend, seasonal and residual components.", d_in(), vec![port("Components", Data)], vec![col("value", "Series", Numeric), int("period", "Season length", 2, 366, 12), choice("model", "Model", &[("additive", "Additive"), ("multiplicative", "Multiplicative"), ("stl", "STL-like (robust iterative)")])]),
        w("stats.acf", "Autocorrelation", Statistics, "ACF and PACF with significance bands.", d_in(), vec![], vec![col("value", "Series", Numeric), int("lags", "Lags", 1, 200, 24), int("diff", "Differencing", 0, 2, 0)]),
        w(
            "stats.arima",
            "ARIMA Forecast",
            Statistics,
            "ARIMA(p,d,q) by conditional sum of squares, with forecasts and prediction intervals.",
            d_in(),
            vec![port("Forecast", Data)],
            vec![col("value", "Series", Numeric), ocol("time", "Time labels", Any), boolean("auto", "Choose p, q by AIC", false), int("p", "AR order (p)", 0, 8, 1), int("d", "Differencing (d)", 0, 2, 1), int("q", "MA order (q)", 0, 8, 1), boolean("constant", "Include constant / drift", true), int("horizon", "Forecast steps", 1, 500, 12), float("level", "Interval level", 0.5, 0.99, 0.95)],
        ),
        w("stats.ets", "Exponential Smoothing", Statistics, "Simple, Holt and Holt–Winters smoothing with forecasts.", d_in(), vec![port("Forecast", Data)], vec![col("value", "Series", Numeric), choice("method", "Model", &[("holt_winters", "Holt–Winters (additive)"), ("holt_winters_mul", "Holt–Winters (multiplicative)"), ("holt", "Holt (trend)"), ("simple", "Simple")]), int("period", "Season length", 2, 366, 12), int("horizon", "Forecast steps", 1, 500, 12), float("level", "Interval level", 0.5, 0.99, 0.95)]),
        w("stats.distfit", "Distribution Fitting", Statistics, "Fits normal, log-normal, exponential, gamma, Weibull, uniform and Poisson by maximum likelihood.", d_in(), vec![], vec![col("column", "Variable", Numeric)]),
        // ---------------------------------------------------------------- Deploy
        w("deploy.register", "Register Model", Deploy, "Saves the trained model to the model registry as a new version.", vec![port("Model", Model), opt("Evaluation Results", Eval)], vec![port("Model", Model)], vec![text("name", "Registered name", ""), text("description", "Description", "")]),
        w("deploy.score", "Batch Score", Deploy, "Applies a registered model to data (new prediction columns).", d_in(), vec![port("Scored Data", Data)], vec![p("model", "Registered model", ParamKind::Registered, J::from("")), text("version", "Version (empty = serving)", ""), boolean("probs", "Class probabilities", true)]),
        w("deploy.drift", "Drift Monitor", Deploy, "Compares new data against the training data of a registered model (PSI, KS) and the prediction distribution.", d_in(), vec![port("Drift Report", Data)], vec![p("model", "Registered model", ParamKind::Registered, J::from("")), text("version", "Version (empty = serving)", "")]),
        w("deploy.save", "Save as Table", Deploy, "Saves the data as a table of the model (a data sheet) for reports.", d_in(), vec![], vec![text("name", "Table name", "Scored")]),
    ]
}

thread_local! {
    static CATALOG: Vec<WidgetType> = catalog();
}

pub fn widget_type(id: &str) -> Option<WidgetType> {
    CATALOG.with(|c| c.iter().find(|t| t.id == id).cloned())
}

/// Reads a setting of a widget (falling back to the catalog default).
pub fn setting(w: &StudioWidget, key: &str) -> J {
    if let Some(v) = w.settings.get(key) {
        return v.clone();
    }
    widget_type(&w.kind).and_then(|t| t.params.iter().find(|p| p.key == key).map(|p| p.default.clone())).unwrap_or(J::Null)
}

pub fn s_str(w: &StudioWidget, key: &str) -> String {
    match setting(w, key) {
        J::String(s) => s,
        J::Null => String::new(),
        v => v.to_string(),
    }
}

pub fn s_f64(w: &StudioWidget, key: &str) -> f64 {
    let v = setting(w, key);
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).unwrap_or(0.0)
}

pub fn s_usize(w: &StudioWidget, key: &str) -> usize {
    s_f64(w, key).max(0.0) as usize
}

pub fn s_bool(w: &StudioWidget, key: &str) -> bool {
    match setting(w, key) {
        J::Bool(b) => b,
        J::Number(n) => n.as_f64().is_some_and(|v| v != 0.0),
        J::String(s) => s == "true",
        _ => false,
    }
}

pub fn s_list(w: &StudioWidget, key: &str) -> Vec<String> {
    match setting(w, key) {
        J::Array(a) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).filter(|s| !s.is_empty()).collect(),
        J::String(s) if !s.is_empty() => s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        _ => vec![],
    }
}

/// A unique workflow name.
pub fn unique_name(existing: &[StudioWorkflow], base: &str) -> String {
    let mut n = base.to_string();
    let mut i = 2;
    while existing.iter().any(|w| w.name == n) {
        n = format!("{base} {i}");
        i += 1;
    }
    n
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_consistent() {
        let c = catalog();
        let mut ids: Vec<&str> = c.iter().map(|t| t.id).collect();
        ids.sort();
        let n = ids.len();
        ids.dedup();
        assert_eq!(n, ids.len(), "duplicate widget ids");
        assert!(n >= 55);
        for cat in Category::ALL {
            assert!(c.iter().any(|t| t.category == cat));
        }
    }

    #[test]
    fn links_and_cycles() {
        let mut wf = StudioWorkflow::default();
        let a = wf.add("data.table", 0.0, 0.0);
        let b = wf.add("model.tree", 100.0, 0.0);
        let c = wf.add("eval.test_score", 200.0, 0.0);
        assert!(wf.connect(a, 0, b, 0).is_ok());
        assert!(wf.connect(b, 0, c, 1).is_ok());
        assert!(wf.connect(a, 0, c, 0).is_ok());
        // Model → Data port is a type error.
        assert!(wf.connect(b, 0, c, 0).is_err());
        let d = wf.add("data.datatable", 0.0, 0.0);
        assert!(wf.connect(a, 0, d, 0).is_ok());
        assert!(wf.connect(d, 0, a, 0).is_err());
        assert_eq!(wf.widget(b).unwrap().label, "Tree");
    }
}
