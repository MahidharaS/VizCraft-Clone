//! Flows: Fabric / KNIME / Orange style data pipelines made of nodes.
//!
//! A [`Flow`] is a DAG of [`FlowNode`]s joined by links (from node/port → to node/port). The
//! [`catalog`] describes every node type (category, ports, settings schema with defaults); the
//! [`Executor`] evaluates nodes on demand, caching each node's output under a hash of its
//! settings, its upstream hashes and its source data, so only what changed re-runs.
//! "Save as model table" nodes become model tables through [`table_for`]
//! (`TableSource::Flow("flow::node")`).

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use serde_json::{Value as J, json};
use vizcraft_query::m::eval::Host;

use crate::build::{ColType, TableData};
use crate::doc::{Flow, FlowNode, Project, TableSource};
use crate::ml;
use crate::scalar::{Scalar, compare};

// =============================================================================================
// Catalog
// =============================================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortKind {
    Table,
    Model,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub name: &'static str,
    pub kind: PortKind,
    pub optional: bool,
}

const fn table(name: &'static str) -> Port {
    Port { name, kind: PortKind::Table, optional: false }
}
const fn opt_table(name: &'static str) -> Port {
    Port { name, kind: PortKind::Table, optional: true }
}
const fn model(name: &'static str) -> Port {
    Port { name, kind: PortKind::Model, optional: false }
}

/// The kind of a setting (drives the inspector form).
#[derive(Clone, Debug, PartialEq)]
pub enum ParamKind {
    Text,
    /// Multi-line code in a language ("python", "formula").
    Code(&'static str),
    Number { min: f64, max: f64, step: f64 },
    Bool,
    Choice(&'static [(&'static str, &'static str)]),
    /// One column of input port N.
    Column(usize),
    /// Several columns of input port N (empty = automatic).
    Columns(usize),
    /// A query, data sheet or model table name.
    Query,
    Sheet,
    ModelTable,
    /// A file path (open / save).
    FileOpen,
    FileSave,
    /// Filter conditions: `[{column, op, value}]`.
    Conditions,
    /// Aggregations: `[{column, op, name}]`.
    Aggregates,
    /// Sort keys: `[{column, desc}]`.
    SortKeys,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: ParamKind,
    pub default: J,
    pub help: &'static str,
}

fn p(key: &'static str, label: &'static str, kind: ParamKind, default: J, help: &'static str) -> Param {
    Param { key, label, kind, default, help }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeType {
    pub id: &'static str,
    pub category: &'static str,
    pub label: &'static str,
    /// Icon name for the UI.
    pub icon: &'static str,
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
    pub params: Vec<Param>,
    pub help: &'static str,
}

impl NodeType {
    pub fn param(&self, key: &str) -> Option<&Param> {
        self.params.iter().find(|p| p.key == key)
    }
    /// Default settings.
    pub fn defaults(&self) -> BTreeMap<String, J> {
        self.params.iter().map(|p| (p.key.to_string(), p.default.clone())).collect()
    }
}

/// Node categories in palette order.
pub const CATEGORIES: &[&str] = &["Data", "Transform", "Unsupervised", "Supervised", "Statistics", "Scripting", "Output"];

const AGG_OPS: &[(&str, &str)] = &[("sum", "Sum"), ("mean", "Average"), ("median", "Median"), ("min", "Min"), ("max", "Max"), ("count", "Count rows"), ("count_distinct", "Count distinct"), ("std", "Std. dev."), ("first", "First"), ("last", "Last")];
pub const FILTER_OPS: &[(&str, &str)] = &[("=", "equals"), ("<>", "does not equal"), (">", "is greater than"), (">=", "is at least"), ("<", "is less than"), ("<=", "is at most"), ("contains", "contains"), ("not_contains", "does not contain"), ("starts", "starts with"), ("ends", "ends with"), ("blank", "is blank"), ("not_blank", "is not blank")];
pub const AGGREGATIONS: &[(&str, &str)] = AGG_OPS;

/// Every node type.
pub fn catalog() -> Vec<NodeType> {
    use ParamKind::*;
    let num = |min: f64, max: f64, step: f64| Number { min, max, step };
    let learner_common = |extra: Vec<Param>| {
        let mut v = vec![
            p("target", "Target", Column(0), json!(""), "The column to predict."),
            p("features", "Features", Columns(0), json!([]), "Input columns (empty: every numeric column except the target)."),
        ];
        v.extend(extra);
        v
    };
    vec![
        // ---------------------------------------------------------------- Data
        NodeType { id: "source.query", category: "Data", label: "Query", icon: "query", inputs: vec![], outputs: vec![table("data")], params: vec![p("name", "Query", Query, json!(""), "A Power Query query's output.")], help: "Reads the output of a Power Query query." },
        NodeType { id: "source.sheet", category: "Data", label: "Data sheet", icon: "sheet", inputs: vec![], outputs: vec![table("data")], params: vec![p("name", "Sheet", Sheet, json!(""), "An Excel-like data sheet.")], help: "Reads a data sheet (evaluated formulas)." },
        NodeType { id: "source.table", category: "Data", label: "Model table", icon: "table", inputs: vec![], outputs: vec![table("data")], params: vec![p("name", "Table", ModelTable, json!(""), "A table of the semantic model.")], help: "Reads a table of the model (as loaded)." },
        NodeType {
            id: "source.csv",
            category: "Data",
            label: "CSV file",
            icon: "csv",
            inputs: vec![],
            outputs: vec![table("data")],
            params: vec![p("path", "File", FileOpen, json!(""), "A .csv / .tsv / .txt file."), p("delimiter", "Delimiter", Choice(&[("auto", "Detect"), (",", "Comma"), (";", "Semicolon"), ("\t", "Tab"), ("|", "Pipe")]), json!("auto"), ""), p("header", "First row has headers", Bool, json!(true), "")],
            help: "Reads a delimited text file.",
        },
        // ---------------------------------------------------------------- Transform
        NodeType { id: "tf.select", category: "Transform", label: "Select columns", icon: "columns", inputs: vec![table("data")], outputs: vec![table("data")], params: vec![p("columns", "Columns", Columns(0), json!([]), ""), p("mode", "Mode", Choice(&[("keep", "Keep these"), ("remove", "Remove these")]), json!("keep"), "")], help: "Keeps or removes columns." },
        NodeType { id: "tf.filter", category: "Transform", label: "Filter rows", icon: "filter", inputs: vec![table("data")], outputs: vec![table("matching"), table("other")], params: vec![p("conditions", "Conditions", Conditions, json!([]), ""), p("match", "Match", Choice(&[("all", "All conditions"), ("any", "Any condition")]), json!("all"), "")], help: "Keeps the rows that meet the conditions (the rest go to the second output)." },
        NodeType { id: "tf.sort", category: "Transform", label: "Sort", icon: "sort", inputs: vec![table("data")], outputs: vec![table("data")], params: vec![p("keys", "Sort by", SortKeys, json!([]), "")], help: "Sorts rows by one or more columns." },
        NodeType { id: "tf.group", category: "Transform", label: "Group by", icon: "group", inputs: vec![table("data")], outputs: vec![table("data")], params: vec![p("by", "Group by", Columns(0), json!([]), "Leave empty to aggregate the whole table."), p("aggregates", "Aggregations", Aggregates, json!([]), "")], help: "Groups rows and aggregates columns." },
        NodeType {
            id: "tf.join",
            category: "Transform",
            label: "Join",
            icon: "join",
            inputs: vec![table("left"), table("right")],
            outputs: vec![table("data")],
            params: vec![
                p("left_keys", "Left keys", Columns(0), json!([]), "Empty: columns with the same name on both sides."),
                p("right_keys", "Right keys", Columns(1), json!([]), ""),
                p("kind", "Join kind", Choice(&[("inner", "Inner"), ("left", "Left outer"), ("right", "Right outer"), ("full", "Full outer")]), json!("inner"), ""),
            ],
            help: "Joins two tables on key columns.",
        },
        NodeType { id: "tf.concat", category: "Transform", label: "Concatenate", icon: "concat", inputs: vec![table("first"), table("second")], outputs: vec![table("data")], params: vec![p("mode", "Columns", Choice(&[("union", "All columns"), ("intersection", "Common columns only")]), json!("union"), "")], help: "Stacks the rows of two tables (columns matched by name)." },
        NodeType {
            id: "tf.formula",
            category: "Transform",
            label: "Formula column",
            icon: "formula",
            inputs: vec![table("data")],
            outputs: vec![table("data")],
            params: vec![p("name", "New column", Text, json!("Result"), ""), p("expression", "Formula", Code("formula"), json!("=[Column1] * 2"), "Excel syntax with [Column] references: =IF([Qty] > 2, [Price] * 0.9, [Price])")],
            help: "Adds a column computed per row with an Excel-style formula.",
        },
        NodeType {
            id: "tf.impute",
            category: "Transform",
            label: "Impute missing",
            icon: "impute",
            inputs: vec![table("data")],
            outputs: vec![table("data")],
            params: vec![p("columns", "Columns", Columns(0), json!([]), "Empty: every column."), p("method", "Replace with", Choice(&[("mean", "Mean"), ("median", "Median"), ("mode", "Most frequent"), ("constant", "A value"), ("drop", "Remove the row")]), json!("mean"), ""), p("value", "Value", Text, json!("0"), "For “A value”.")],
            help: "Fills in blank values.",
        },
        NodeType { id: "tf.normalize", category: "Transform", label: "Normalize", icon: "normalize", inputs: vec![table("data")], outputs: vec![table("data")], params: vec![p("columns", "Columns", Columns(0), json!([]), "Empty: numeric columns."), p("method", "Method", Choice(&[("zscore", "Standardize (z-score)"), ("minmax", "Scale to 0–1")]), json!("zscore"), "")], help: "Rescales numeric columns." },
        NodeType {
            id: "tf.bin",
            category: "Transform",
            label: "Discretize",
            icon: "bin",
            inputs: vec![table("data")],
            outputs: vec![table("data")],
            params: vec![p("column", "Column", Column(0), json!(""), ""), p("bins", "Bins", num(2.0, 50.0, 1.0), json!(4), ""), p("method", "Method", Choice(&[("width", "Equal width"), ("quantile", "Equal frequency")]), json!("width"), ""), p("name", "New column", Text, json!(""), "Empty: “<column> bin”.")],
            help: "Turns a numeric column into ranges.",
        },
        NodeType {
            id: "tf.sample",
            category: "Transform",
            label: "Sample",
            icon: "sample",
            inputs: vec![table("data")],
            outputs: vec![table("sample"), table("remaining")],
            params: vec![p("mode", "Mode", Choice(&[("random", "Random rows"), ("fraction", "Random fraction"), ("first", "First rows")]), json!("random"), ""), p("n", "Rows", num(1.0, 1e7, 1.0), json!(100), ""), p("fraction", "Fraction", num(0.0, 1.0, 0.05), json!(0.2), ""), p("seed", "Seed", num(0.0, 1e9, 1.0), json!(42), "")],
            help: "Takes a subset of rows.",
        },
        NodeType {
            id: "tf.pivot",
            category: "Transform",
            label: "Pivot",
            icon: "pivot",
            inputs: vec![table("data")],
            outputs: vec![table("data")],
            params: vec![p("rows", "Row keys", Columns(0), json!([]), ""), p("column", "Pivot column", Column(0), json!(""), "Its values become columns."), p("value", "Values", Column(0), json!(""), ""), p("op", "Aggregation", Choice(AGG_OPS), json!("sum"), "")],
            help: "Spreads the values of a column into new columns.",
        },
        NodeType { id: "tf.unpivot", category: "Transform", label: "Unpivot", icon: "unpivot", inputs: vec![table("data")], outputs: vec![table("data")], params: vec![p("keep", "Keep columns", Columns(0), json!([]), "The other columns turn into Attribute/Value rows."), p("attribute", "Attribute name", Text, json!("Attribute"), ""), p("value", "Value name", Text, json!("Value"), "")], help: "Turns columns into rows." },
        NodeType { id: "tf.dedupe", category: "Transform", label: "Remove duplicates", icon: "dedupe", inputs: vec![table("data")], outputs: vec![table("data")], params: vec![p("columns", "Compare columns", Columns(0), json!([]), "Empty: whole rows.")], help: "Keeps the first of rows that repeat." },
        // ---------------------------------------------------------------- Unsupervised
        NodeType {
            id: "ml.kmeans",
            category: "Unsupervised",
            label: "k-Means",
            icon: "cluster",
            inputs: vec![table("data")],
            outputs: vec![table("data"), table("centroids")],
            params: vec![
                p("columns", "Columns", Columns(0), json!([]), "Empty: numeric columns."),
                p("auto", "Choose k automatically", Bool, json!(false), "Tries k = 2…max k and keeps the best silhouette."),
                p("k", "Clusters (k)", num(1.0, 50.0, 1.0), json!(3), ""),
                p("max_k", "Max k", num(2.0, 30.0, 1.0), json!(8), ""),
                p("normalize", "Normalize columns", Bool, json!(true), ""),
                p("seed", "Seed", num(0.0, 1e9, 1.0), json!(42), ""),
                p("output", "Cluster column", Text, json!("Cluster"), ""),
            ],
            help: "Groups rows into k clusters of similar values.",
        },
        NodeType {
            id: "ml.hclust",
            category: "Unsupervised",
            label: "Hierarchical clustering",
            icon: "hclust",
            inputs: vec![table("data")],
            outputs: vec![table("data")],
            params: vec![p("columns", "Columns", Columns(0), json!([]), ""), p("k", "Clusters", num(1.0, 50.0, 1.0), json!(3), "Where to cut the tree."), p("normalize", "Normalize columns", Bool, json!(true), ""), p("output", "Cluster column", Text, json!("Cluster"), "")],
            help: "Agglomerative clustering (average linkage).",
        },
        NodeType {
            id: "ml.dbscan",
            category: "Unsupervised",
            label: "DBSCAN",
            icon: "dbscan",
            inputs: vec![table("data")],
            outputs: vec![table("data")],
            params: vec![p("columns", "Columns", Columns(0), json!([]), ""), p("eps", "Neighbourhood (ε)", num(0.0, 1e6, 0.05), json!(0.5), "In normalized units when normalizing."), p("min_points", "Core point neighbours", num(1.0, 1000.0, 1.0), json!(4), ""), p("normalize", "Normalize columns", Bool, json!(true), ""), p("output", "Cluster column", Text, json!("Cluster"), "")],
            help: "Density-based clusters; sparse points are noise.",
        },
        NodeType {
            id: "ml.pca",
            category: "Unsupervised",
            label: "PCA",
            icon: "pca",
            inputs: vec![table("data")],
            outputs: vec![table("data"), table("variance")],
            params: vec![p("columns", "Columns", Columns(0), json!([]), ""), p("components", "Components", num(1.0, 50.0, 1.0), json!(2), ""), p("standardize", "Standardize", Bool, json!(true), ""), p("keep", "Keep original columns", Bool, json!(true), "")],
            help: "Principal component analysis.",
        },
        // ---------------------------------------------------------------- Supervised
        NodeType {
            id: "ml.split",
            category: "Supervised",
            label: "Train/test split",
            icon: "split",
            inputs: vec![table("data")],
            outputs: vec![table("train"), table("test")],
            params: vec![p("train", "Training share", num(0.05, 0.95, 0.05), json!(0.7), ""), p("stratify", "Stratify by", Column(0), json!(""), "Keeps class proportions (optional)."), p("seed", "Seed", num(0.0, 1e9, 1.0), json!(42), "")],
            help: "Splits rows into training and test sets.",
        },
        NodeType { id: "ml.linreg", category: "Supervised", label: "Linear regression", icon: "linear", inputs: vec![table("data")], outputs: vec![model("model"), table("fitted")], params: learner_common(vec![]), help: "Ordinary least squares: coefficients, R²." },
        NodeType {
            id: "ml.logreg",
            category: "Supervised",
            label: "Logistic regression",
            icon: "logistic",
            inputs: vec![table("data")],
            outputs: vec![model("model"), table("fitted")],
            params: learner_common(vec![p("iterations", "Iterations", num(10.0, 10000.0, 10.0), json!(400), ""), p("learning_rate", "Learning rate", num(0.001, 5.0, 0.01), json!(0.3), ""), p("l2", "Regularization (L2)", num(0.0, 10.0, 0.001), json!(0.001), "")]),
            help: "Classification by gradient descent (one-vs-rest for many classes).",
        },
        NodeType {
            id: "ml.tree",
            category: "Supervised",
            label: "Decision tree",
            icon: "tree",
            inputs: vec![table("data")],
            outputs: vec![model("model"), table("fitted")],
            params: learner_common(vec![p("task", "Task", Choice(&[("auto", "Automatic"), ("classification", "Classification"), ("regression", "Regression")]), json!("auto"), ""), p("max_depth", "Max depth", num(1.0, 30.0, 1.0), json!(5), ""), p("min_leaf", "Min rows per leaf", num(1.0, 1000.0, 1.0), json!(2), "")]),
            help: "CART tree for classification or regression.",
        },
        NodeType {
            id: "ml.forest",
            category: "Supervised",
            label: "Random forest",
            icon: "forest",
            inputs: vec![table("data")],
            outputs: vec![model("model"), table("fitted")],
            params: learner_common(vec![p("task", "Task", Choice(&[("auto", "Automatic"), ("classification", "Classification"), ("regression", "Regression")]), json!("auto"), ""), p("trees", "Trees", num(1.0, 500.0, 1.0), json!(50), ""), p("max_depth", "Max depth", num(1.0, 30.0, 1.0), json!(8), ""), p("seed", "Seed", num(0.0, 1e9, 1.0), json!(42), "")]),
            help: "Bagged decision trees with random feature subsets.",
        },
        NodeType { id: "ml.nb", category: "Supervised", label: "Naive Bayes", icon: "bayes", inputs: vec![table("data")], outputs: vec![model("model"), table("fitted")], params: learner_common(vec![]), help: "Gaussian naive Bayes classifier." },
        NodeType {
            id: "ml.knn",
            category: "Supervised",
            label: "kNN",
            icon: "knn",
            inputs: vec![table("data")],
            outputs: vec![model("model"), table("fitted")],
            params: learner_common(vec![p("task", "Task", Choice(&[("auto", "Automatic"), ("classification", "Classification"), ("regression", "Regression")]), json!("auto"), ""), p("k", "Neighbours (k)", num(1.0, 200.0, 1.0), json!(5), "")]),
            help: "k-nearest neighbours (standardized features).",
        },
        NodeType { id: "ml.predict", category: "Supervised", label: "Predict", icon: "predict", inputs: vec![model("model"), table("data")], outputs: vec![table("data")], params: vec![p("output", "Prediction column", Text, json!("Prediction"), ""), p("probabilities", "Add class probabilities", Bool, json!(true), "")], help: "Applies a trained model to new rows." },
        NodeType { id: "ml.score", category: "Supervised", label: "Test & Score", icon: "score", inputs: vec![model("model"), table("test")], outputs: vec![table("metrics"), table("predictions")], params: vec![], help: "Evaluates a model on test data: accuracy, precision, recall, F1, AUC or RMSE, MAE, R²." },
        NodeType { id: "ml.confusion", category: "Supervised", label: "Confusion matrix", icon: "confusion", inputs: vec![table("data")], outputs: vec![table("matrix")], params: vec![p("actual", "Actual", Column(0), json!(""), ""), p("predicted", "Predicted", Column(0), json!("Prediction"), "")], help: "Counts of actual vs predicted classes." },
        // ---------------------------------------------------------------- Statistics
        NodeType {
            id: "stat.outliers",
            category: "Statistics",
            label: "Outliers",
            icon: "outlier",
            inputs: vec![table("data")],
            outputs: vec![table("data")],
            params: vec![
                p("columns", "Columns", Columns(0), json!([]), ""),
                p("method", "Method", Choice(&[("zscore", "z-score"), ("iqr", "Interquartile range"), ("isolation", "Isolation forest")]), json!("zscore"), ""),
                p("threshold", "Threshold", num(0.0, 100.0, 0.1), json!(3.0), "z: |z| above; IQR: IQRs beyond the quartiles; isolation: score (0.5–1)."),
                p("action", "Rows", Choice(&[("flag", "Flag all rows"), ("remove", "Remove outliers"), ("only", "Only outliers")]), json!("flag"), ""),
            ],
            help: "Finds unusual rows.",
        },
        NodeType { id: "stat.corr", category: "Statistics", label: "Correlations", icon: "corr", inputs: vec![table("data")], outputs: vec![table("matrix")], params: vec![p("columns", "Columns", Columns(0), json!([]), "")], help: "Pearson correlation matrix." },
        NodeType { id: "stat.stats", category: "Statistics", label: "Feature statistics", icon: "stats", inputs: vec![table("data")], outputs: vec![table("statistics")], params: vec![], help: "Count, missing, mean, quartiles… per column." },
        NodeType { id: "stat.dist", category: "Statistics", label: "Distribution", icon: "dist", inputs: vec![table("data")], outputs: vec![table("bins")], params: vec![p("column", "Column", Column(0), json!(""), ""), p("bins", "Bins", num(2.0, 100.0, 1.0), json!(12), "")], help: "Histogram of a numeric column or counts of a categorical one." },
        // ---------------------------------------------------------------- Scripting
        NodeType {
            id: "script.python",
            category: "Scripting",
            label: "Python script",
            icon: "python",
            inputs: vec![opt_table("in_df"), opt_table("in_df2")],
            outputs: vec![table("out_df")],
            params: vec![p("code", "Script", Code("python"), json!("# in_df is a pandas DataFrame (or a list of dicts without pandas).\n# Assign the result to out_df.\nout_df = in_df\n"), "")],
            help: "Runs python3 with in_df / in_df2; returns out_df.",
        },
        // ---------------------------------------------------------------- Output
        NodeType { id: "out.table", category: "Output", label: "Save as model table", icon: "save", inputs: vec![table("data")], outputs: vec![table("data")], params: vec![p("name", "Table name", Text, json!(""), "The model table this output becomes (Add to model).")], help: "Makes the data a table of the model (like a Fabric lakehouse table)." },
        NodeType { id: "out.csv", category: "Output", label: "Export CSV", icon: "export", inputs: vec![table("data")], outputs: vec![table("data")], params: vec![p("path", "File", FileSave, json!(""), "Written on each run.")], help: "Writes the data to a CSV file." },
    ]
}

pub fn node_type(id: &str) -> Option<NodeType> {
    catalog().into_iter().find(|n| n.id == id)
}

/// A setting of a node (or its default).
pub fn setting(node: &FlowNode, key: &str) -> J {
    if let Some(v) = node.config.get(key) {
        return v.clone();
    }
    node_type(&node.kind).and_then(|t| t.param(key).map(|p| p.default.clone())).unwrap_or(J::Null)
}

fn s_str(node: &FlowNode, key: &str) -> String {
    match setting(node, key) {
        J::String(s) => s,
        J::Null => String::new(),
        other => other.to_string(),
    }
}
fn s_num(node: &FlowNode, key: &str) -> f64 {
    match setting(node, key) {
        J::Number(n) => n.as_f64().unwrap_or(0.0),
        J::String(s) => s.trim().parse().unwrap_or(0.0),
        J::Bool(b) => f64::from(u8::from(b)),
        _ => 0.0,
    }
}
fn s_bool(node: &FlowNode, key: &str) -> bool {
    match setting(node, key) {
        J::Bool(b) => b,
        J::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        J::String(s) => s == "true",
        _ => false,
    }
}
fn s_list(node: &FlowNode, key: &str) -> Vec<String> {
    match setting(node, key) {
        J::Array(a) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).filter(|s| !s.is_empty()).collect(),
        J::String(s) if !s.is_empty() => s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        _ => vec![],
    }
}
fn s_objs(node: &FlowNode, key: &str) -> Vec<serde_json::Map<String, J>> {
    match setting(node, key) {
        J::Array(a) => a.into_iter().filter_map(|v| if let J::Object(o) = v { Some(o) } else { None }).collect(),
        _ => vec![],
    }
}
fn o_str(o: &serde_json::Map<String, J>, k: &str) -> String {
    match o.get(k) {
        Some(J::String(s)) => s.clone(),
        Some(J::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

// =============================================================================================
// Results
// =============================================================================================

#[derive(Clone, Debug)]
pub enum PortData {
    Table(Rc<TableData>),
    Model(Rc<ml::Model>),
}

impl PortData {
    pub fn table(&self) -> Option<&Rc<TableData>> {
        match self {
            PortData::Table(t) => Some(t),
            _ => None,
        }
    }
}

/// A scatter plot for the preview (points coloured by group).
#[derive(Clone, Debug, Default)]
pub struct Scatter {
    pub x_label: String,
    pub y_label: String,
    pub points: Vec<(f64, f64, usize)>,
    pub groups: Vec<String>,
    pub centers: Vec<(f64, f64)>,
}

/// Bars for the preview (cluster sizes, explained variance, histograms).
#[derive(Clone, Debug, Default)]
pub struct Bars {
    pub title: String,
    pub labels: Vec<String>,
    pub values: Vec<f64>,
}

/// What a node shows besides its output table: metrics, extra tables, charts, logs.
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub metrics: Vec<(String, String)>,
    pub tables: Vec<(String, Rc<TableData>)>,
    pub scatter: Option<Scatter>,
    pub bars: Option<Bars>,
    /// Text output (Python stdout / stderr, notes).
    pub log: String,
    /// A matrix to draw as a heatmap (correlations, confusion): labels and values.
    pub heatmap: Option<(Vec<String>, Vec<String>, Vec<Vec<f64>>)>,
}

#[derive(Clone, Debug, Default)]
pub struct NodeOutput {
    pub ports: Vec<PortData>,
    pub summary: Summary,
    pub ms: f64,
}

impl NodeOutput {
    pub fn table(&self, port: usize) -> Option<&Rc<TableData>> {
        self.ports.get(port).and_then(PortData::table)
    }
}

/// Evaluation context: the project, the host (files, web, databases) and optionally the
/// already-loaded model tables.
pub struct Ctx<'a> {
    pub project: &'a Project,
    pub host: &'a dyn Host,
    pub tables: Option<&'a BTreeMap<String, Rc<TableData>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Not run (or its inputs/settings changed since).
    Stale,
    Ok,
    Error,
}

/// Runs flows with a per-node cache.
#[derive(Default)]
pub struct Executor {
    cache: HashMap<(String, u64), (u64, Result<Rc<NodeOutput>, String>)>,
}

thread_local! {
    static FLOW_DEPTH: Cell<usize> = const { Cell::new(0) };
}

impl Executor {
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// The cached result of a node, if any (even stale).
    pub fn cached(&self, flow: &str, node: u64) -> Option<&Result<Rc<NodeOutput>, String>> {
        self.cache.get(&(flow.to_string(), node)).map(|(_, r)| r)
    }

    /// Statuses of every node (hash comparison; nothing runs).
    pub fn statuses(&self, ctx: &Ctx, flow: &Flow) -> HashMap<u64, Status> {
        let mut memo = HashMap::new();
        flow.nodes
            .iter()
            .map(|n| {
                let h = node_hash(ctx, flow, n.id, &mut memo, 0);
                let st = match self.cache.get(&(flow.name.clone(), n.id)) {
                    Some((ch, r)) if *ch == h => {
                        if r.is_ok() {
                            Status::Ok
                        } else {
                            Status::Error
                        }
                    }
                    _ => Status::Stale,
                };
                (n.id, st)
            })
            .collect()
    }

    /// Evaluates a node (and what it needs upstream).
    pub fn run(&mut self, ctx: &Ctx, flow: &Flow, node: u64) -> Result<Rc<NodeOutput>, String> {
        let mut memo = HashMap::new();
        let mut visiting = HashSet::new();
        self.run_inner(ctx, flow, node, &mut memo, &mut visiting)
    }

    /// Evaluates every node of a flow; returns the number of errors.
    pub fn run_all(&mut self, ctx: &Ctx, flow: &Flow) -> usize {
        let mut memo = HashMap::new();
        let mut errors = 0;
        for n in &flow.nodes {
            let mut visiting = HashSet::new();
            if self.run_inner(ctx, flow, n.id, &mut memo, &mut visiting).is_err() {
                errors += 1;
            }
        }
        errors
    }

    fn run_inner(&mut self, ctx: &Ctx, flow: &Flow, id: u64, memo: &mut HashMap<u64, u64>, visiting: &mut HashSet<u64>) -> Result<Rc<NodeOutput>, String> {
        let Some(node) = flow.nodes.iter().find(|n| n.id == id) else { return Err("The node is missing.".into()) };
        if !visiting.insert(id) {
            return Err("The flow has a cycle.".into());
        }
        let h = node_hash(ctx, flow, id, memo, 0);
        let key = (flow.name.clone(), id);
        if let Some((ch, r)) = self.cache.get(&key)
            && *ch == h
        {
            visiting.remove(&id);
            return r.clone();
        }
        let Some(nt) = node_type(&node.kind) else {
            visiting.remove(&id);
            return Err(format!("Unknown node type `{}`.", node.kind));
        };
        let mut inputs: Vec<Option<PortData>> = Vec::new();
        let mut failure = None;
        for (pi, port) in nt.inputs.iter().enumerate() {
            match flow.links.iter().find(|l| l.2 == id && l.3 == pi) {
                Some(&(from, fport, _, _)) => match self.run_inner(ctx, flow, from, memo, visiting) {
                    Ok(out) => match out.ports.get(fport) {
                        Some(pd) => inputs.push(Some(pd.clone())),
                        None => {
                            failure = Some(format!("The {} input is connected to a missing output.", port.name));
                            break;
                        }
                    },
                    Err(_) => {
                        let label = flow.nodes.iter().find(|n| n.id == from).map(|n| n.label.clone()).unwrap_or_default();
                        failure = Some(format!("The upstream node “{label}” failed."));
                        break;
                    }
                },
                None if port.optional => inputs.push(None),
                None => {
                    failure = Some(format!("Connect the “{}” input.", port.name));
                    break;
                }
            }
        }
        let t0 = std::time::Instant::now();
        let result = match failure {
            Some(f) => Err(f),
            None => exec(ctx, flow, node, &inputs).map(|mut o| {
                o.ms = t0.elapsed().as_secs_f64() * 1000.0;
                Rc::new(o)
            }),
        };
        self.cache.insert(key, (h, result.clone()));
        visiting.remove(&id);
        result
    }
}

fn hash_str(h: &mut impl Hasher, s: &str) {
    s.hash(h);
}

/// Hash of a node: kind, settings, upstream hashes and (for sources) the source data.
pub fn node_hash(ctx: &Ctx, flow: &Flow, id: u64, memo: &mut HashMap<u64, u64>, depth: usize) -> u64 {
    if let Some(h) = memo.get(&id) {
        return *h;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let Some(node) = flow.nodes.iter().find(|n| n.id == id) else { return 0 };
    hash_str(&mut h, &node.kind);
    hash_str(&mut h, &serde_json::to_string(&node.config).unwrap_or_default());
    if depth > 200 {
        return 1;
    }
    let mut links: Vec<&(u64, usize, u64, usize)> = flow.links.iter().filter(|l| l.2 == id).collect();
    links.sort_by_key(|l| l.3);
    for l in links {
        (l.3, l.1).hash(&mut h);
        node_hash(ctx, flow, l.0, memo, depth + 1).hash(&mut h);
    }
    // Source data fingerprints.
    let name = s_str(node, "name");
    match node.kind.as_str() {
        "source.sheet" => {
            if let Some(s) = ctx.project.sheet(&name) {
                s.cells.hash(&mut h);
                s.col_types.hash(&mut h);
            }
            // Cross-sheet references.
            ctx.project.sheets.len().hash(&mut h);
        }
        "source.query" => {
            hash_str(&mut h, &serde_json::to_string(&ctx.project.queries).unwrap_or_default());
            for s in &ctx.project.sheets {
                s.cells.len().hash(&mut h);
            }
        }
        "source.table" => match ctx.tables.and_then(|t| t.get(&name)) {
            Some(t) => {
                (Rc::as_ptr(t) as usize).hash(&mut h);
                t.rows.len().hash(&mut h);
            }
            None => hash_str(&mut h, &serde_json::to_string(&ctx.project.model.tables).unwrap_or_default()),
        },
        "source.csv" => {
            if let Ok(m) = std::fs::metadata(s_str(node, "path")) {
                m.len().hash(&mut h);
                if let Ok(t) = m.modified() {
                    t.hash(&mut h);
                }
            }
        }
        "out.csv" | "script.python" => {}
        _ => {}
    }
    let v = h.finish();
    memo.insert(id, v);
    v
}

// =============================================================================================
// Node execution
// =============================================================================================

fn in_table(inputs: &[Option<PortData>], i: usize) -> Result<Rc<TableData>, String> {
    match inputs.get(i) {
        Some(Some(PortData::Table(t))) => Ok(t.clone()),
        Some(Some(PortData::Model(_))) => Err("Expected a table, got a model.".into()),
        _ => Err("An input is missing.".into()),
    }
}

fn out(t: TableData) -> NodeOutput {
    NodeOutput { ports: vec![PortData::Table(Rc::new(t))], ..NodeOutput::default() }
}

fn col_idx(t: &TableData, name: &str) -> Result<usize, String> {
    t.column_index(name).ok_or_else(|| format!("There's no column `{name}`."))
}

fn exec(ctx: &Ctx, flow: &Flow, node: &FlowNode, inputs: &[Option<PortData>]) -> Result<NodeOutput, String> {
    let _ = flow;
    let k = node.kind.as_str();
    match k {
        // ------------------------------------------------------------ sources
        "source.query" => {
            let name = s_str(node, "name");
            if name.is_empty() {
                return Err("Choose a query.".into());
            }
            let t = crate::build::evaluate_query(ctx.project, &name, ctx.host).map_err(|e| e.to_string())?;
            Ok(out(TableData::from_m(&name, &t)))
        }
        "source.sheet" => {
            let name = s_str(node, "name");
            let s = ctx.project.sheet(&name).ok_or_else(|| if name.is_empty() { "Choose a data sheet.".to_string() } else { format!("There's no data sheet `{name}`.") })?;
            Ok(out(crate::sheet::evaluate_with(s, &ctx.project.sheets)))
        }
        "source.table" => {
            let name = s_str(node, "name");
            if name.is_empty() {
                return Err("Choose a model table.".into());
            }
            if let Some(t) = ctx.tables.and_then(|m| m.get(&name).or_else(|| m.iter().find(|(k, _)| k.eq_ignore_ascii_case(&name)).map(|(_, v)| v))) {
                return Ok(NodeOutput { ports: vec![PortData::Table(t.clone())], ..NodeOutput::default() });
            }
            let mt = ctx.project.model.tables.iter().find(|t| t.name.eq_ignore_ascii_case(&name)).ok_or_else(|| format!("There's no model table `{name}`."))?;
            let data = match &mt.source {
                TableSource::Query(q) => crate::build::evaluate_query(ctx.project, q, ctx.host).map(|m| TableData::from_m(&name, &m)).map_err(|e| e.to_string())?,
                TableSource::Sheet(s) => ctx.project.sheet(s).map(|sh| crate::sheet::evaluate_with(sh, &ctx.project.sheets)).ok_or_else(|| format!("The data sheet `{s}` is missing."))?,
                TableSource::Flow(spec) => table_for(ctx.project, spec, ctx.host).map_err(|e| e.to_string())?,
                _ => return Err(format!("`{name}` is a calculated table: refresh the model, then use it here.")),
            };
            Ok(out(data))
        }
        "source.csv" => {
            let path = s_str(node, "path");
            if path.is_empty() {
                return Err("Choose a CSV file.".into());
            }
            let bytes = ctx.host.file_contents(&path).or_else(|_| std::fs::read(&path).map_err(|e| vizcraft_query::Error::Eval(e.to_string()))).map_err(|e| format!("Couldn't read {path}: {}", e.message()))?;
            let text = String::from_utf8_lossy(&bytes);
            let d = s_str(node, "delimiter");
            let delim = if d == "auto" || d.is_empty() { detect_delimiter(&text) } else { d.chars().next().unwrap_or(',') };
            let rows = parse_csv(&text, delim);
            Ok(out(table_from_rows(&file_stem(&path), rows, s_bool(node, "header"))))
        }
        // ------------------------------------------------------------ transforms
        "tf.select" => {
            let t = in_table(inputs, 0)?;
            let cols = s_list(node, "columns");
            let keep = s_str(node, "mode") != "remove";
            for c in &cols {
                col_idx(&t, c)?;
            }
            let idx: Vec<usize> = if keep { cols.iter().filter_map(|c| t.column_index(c)).collect() } else { (0..t.columns.len()).filter(|i| !cols.iter().any(|c| t.columns.get(*i).is_some_and(|n| n.eq_ignore_ascii_case(c)))).collect() };
            Ok(out(project_cols(&t, &idx)))
        }
        "tf.filter" => {
            let t = in_table(inputs, 0)?;
            let conds = s_objs(node, "conditions");
            let any = s_str(node, "match") == "any";
            let mut compiled = Vec::new();
            for c in &conds {
                let col = o_str(c, "column");
                if col.is_empty() {
                    continue;
                }
                let i = col_idx(&t, &col)?;
                let ty = t.types.get(i).copied().unwrap_or_default();
                compiled.push((i, o_str(c, "op"), condition_value(&o_str(c, "value"), ty)));
            }
            let (mut yes, mut no) = (Vec::new(), Vec::new());
            for r in &t.rows {
                let ok = if compiled.is_empty() {
                    true
                } else if any {
                    compiled.iter().any(|(i, op, v)| test(r.get(*i).unwrap_or(&Scalar::Null), op, v))
                } else {
                    compiled.iter().all(|(i, op, v)| test(r.get(*i).unwrap_or(&Scalar::Null), op, v))
                };
                if ok { yes.push(r.clone()) } else { no.push(r.clone()) }
            }
            let mut o = out(TableData { rows: yes, ..(*t).clone() });
            o.ports.push(PortData::Table(Rc::new(TableData { rows: no, ..(*t).clone() })));
            o.summary.metrics.push(("Matching rows".into(), fmt_int(o.table(0).map(|x| x.rows.len()).unwrap_or(0))));
            o.summary.metrics.push(("Other rows".into(), fmt_int(o.table(1).map(|x| x.rows.len()).unwrap_or(0))));
            Ok(o)
        }
        "tf.sort" => {
            let t = in_table(inputs, 0)?;
            let mut keys = Vec::new();
            for k in s_objs(node, "keys") {
                let c = o_str(&k, "column");
                if c.is_empty() {
                    continue;
                }
                keys.push((col_idx(&t, &c)?, k.get("desc").and_then(J::as_bool).unwrap_or(false)));
            }
            let mut rows = t.rows.clone();
            rows.sort_by(|a, b| {
                for (i, desc) in &keys {
                    let o = compare(a.get(*i).unwrap_or(&Scalar::Null), b.get(*i).unwrap_or(&Scalar::Null));
                    let o = if *desc { o.reverse() } else { o };
                    if o != std::cmp::Ordering::Equal {
                        return o;
                    }
                }
                std::cmp::Ordering::Equal
            });
            Ok(out(TableData { rows, ..(*t).clone() }))
        }
        "tf.group" => {
            let t = in_table(inputs, 0)?;
            let by = s_list(node, "by");
            let aggs: Vec<(String, String, String)> = s_objs(node, "aggregates").iter().map(|o| (o_str(o, "column"), o_str(o, "op"), o_str(o, "name"))).filter(|a| !a.1.is_empty()).collect();
            group_by(&t, &by, &aggs).map(out)
        }
        "tf.join" => {
            let l = in_table(inputs, 0)?;
            let r = in_table(inputs, 1)?;
            let mut lk = s_list(node, "left_keys");
            let mut rk = s_list(node, "right_keys");
            if lk.is_empty() && rk.is_empty() {
                lk = l.columns.iter().filter(|c| r.column_index(c).is_some()).cloned().collect();
                rk = lk.clone();
            } else if rk.is_empty() {
                rk = lk.clone();
            }
            if lk.is_empty() {
                return Err("Choose the key columns to join on.".into());
            }
            if lk.len() != rk.len() {
                return Err("Choose as many right keys as left keys.".into());
            }
            join(&l, &r, &lk, &rk, &s_str(node, "kind")).map(out)
        }
        "tf.concat" => {
            let a = in_table(inputs, 0)?;
            let b = in_table(inputs, 1)?;
            let inter = s_str(node, "mode") == "intersection";
            let mut cols: Vec<String> = a.columns.clone();
            if inter {
                cols.retain(|c| b.column_index(c).is_some());
            } else {
                for c in &b.columns {
                    if !cols.iter().any(|x| x.eq_ignore_ascii_case(c)) {
                        cols.push(c.clone());
                    }
                }
            }
            let pick = |t: &TableData| -> Vec<Vec<Scalar>> { t.rows.iter().map(|r| cols.iter().map(|c| t.column_index(c).and_then(|i| r.get(i)).cloned().unwrap_or(Scalar::Null)).collect()).collect() };
            let mut rows = pick(&a);
            rows.extend(pick(&b));
            let types = cols.iter().map(|c| a.column_index(c).and_then(|i| a.types.get(i).copied()).or_else(|| b.column_index(c).and_then(|i| b.types.get(i).copied())).unwrap_or_default()).collect();
            Ok(out(TableData { name: a.name.clone(), columns: cols, types, rows }))
        }
        "tf.formula" => {
            let t = in_table(inputs, 0)?;
            let name = s_str(node, "name");
            let name = if name.trim().is_empty() { "Result".to_string() } else { name };
            let f = crate::sheet::RowFormula::compile(&s_str(node, "expression"), &t.columns)?;
            let vals: Vec<Scalar> = t.rows.iter().map(|r| f.eval(r).to_scalar()).collect();
            let errors = t.rows.iter().take(2000).filter(|r| f.eval(r).is_error()).count();
            let mut nt = (*t).clone();
            set_column(&mut nt, &name, vals, ColType::Any);
            let mut o = out(nt);
            if errors > 0 {
                o.summary.metrics.push(("Rows with errors (blank)".into(), fmt_int(errors)));
            }
            Ok(o)
        }
        "tf.impute" => {
            let t = in_table(inputs, 0)?;
            let mut cols = s_list(node, "columns");
            if cols.is_empty() {
                cols = t.columns.clone();
            }
            let method = s_str(node, "method");
            let mut nt = (*t).clone();
            let mut filled = 0usize;
            let idx: Vec<usize> = cols.iter().map(|c| col_idx(&t, c)).collect::<Result<_, _>>()?;
            if method == "drop" {
                let before = nt.rows.len();
                nt.rows.retain(|r| idx.iter().all(|i| !r.get(*i).unwrap_or(&Scalar::Null).is_null()));
                let mut o = out(nt);
                o.summary.metrics.push(("Rows removed".into(), fmt_int(before - o.table(0).map(|t| t.rows.len()).unwrap_or(0))));
                return Ok(o);
            }
            for &i in &idx {
                let nums: Vec<f64> = t.rows.iter().filter_map(|r| if let Some(Scalar::Num(n)) = r.get(i) { Some(*n) } else { None }).collect();
                let fill = match method.as_str() {
                    "mean" if !nums.is_empty() => Scalar::Num(ml::mean(&nums)),
                    "median" if !nums.is_empty() => Scalar::Num(ml::quantile_sorted(&ml::sorted_finite(&nums), 0.5)),
                    "constant" => crate::sheet::parse_input(&s_str(node, "value")),
                    _ => {
                        let mut counts: BTreeMap<String, (usize, Scalar)> = BTreeMap::new();
                        for r in &t.rows {
                            if let Some(v) = r.get(i).filter(|v| !v.is_null()) {
                                counts.entry(v.key()).or_insert((0, v.clone())).0 += 1;
                            }
                        }
                        counts.into_values().max_by_key(|(n, _)| *n).map(|(_, v)| v).unwrap_or(Scalar::Null)
                    }
                };
                for r in nt.rows.iter_mut() {
                    if let Some(v) = r.get_mut(i)
                        && v.is_null()
                    {
                        *v = fill.clone();
                        filled += 1;
                    }
                }
            }
            let mut o = out(nt);
            o.summary.metrics.push(("Values filled".into(), fmt_int(filled)));
            Ok(o)
        }
        "tf.normalize" => {
            let t = in_table(inputs, 0)?;
            let mut cols = s_list(node, "columns");
            if cols.is_empty() {
                cols = ml::numeric_columns(&t);
            }
            let minmax = s_str(node, "method") == "minmax";
            let mut nt = (*t).clone();
            for c in &cols {
                let i = col_idx(&t, c)?;
                let v: Vec<f64> = t.rows.iter().filter_map(|r| r.get(i).and_then(Scalar::as_f64)).collect();
                let (a, b) = if minmax {
                    let lo = v.iter().copied().fold(f64::INFINITY, f64::min);
                    let hi = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    (lo, if hi > lo { hi - lo } else { 1.0 })
                } else {
                    let s = ml::std_dev(&v);
                    (ml::mean(&v), if s > 0.0 { s } else { 1.0 })
                };
                for r in nt.rows.iter_mut() {
                    if let Some(x) = r.get_mut(i)
                        && let Some(n) = x.as_f64()
                    {
                        *x = Scalar::Num((n - a) / b);
                    }
                }
                if let Some(ty) = nt.types.get_mut(i) {
                    *ty = ColType::Number;
                }
            }
            Ok(out(nt))
        }
        "tf.bin" => {
            let t = in_table(inputs, 0)?;
            let col = s_str(node, "column");
            let i = col_idx(&t, &col)?;
            let bins = (s_num(node, "bins") as usize).clamp(2, 50);
            let v: Vec<f64> = t.rows.iter().map(|r| r.get(i).and_then(Scalar::as_f64).unwrap_or(f64::NAN)).collect();
            let s = ml::sorted_finite(&v);
            let edges: Vec<f64> = if s_str(node, "method") == "quantile" {
                let mut e: Vec<f64> = (0..=bins).map(|k| ml::quantile_sorted(&s, k as f64 / bins as f64)).collect();
                e.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
                e
            } else {
                let lo = s.first().copied().unwrap_or(0.0);
                let hi = s.last().copied().unwrap_or(0.0);
                (0..=bins).map(|k| lo + (hi - lo) * k as f64 / bins as f64).collect()
            };
            let label = |b: usize| format!("{} – {}", short_num(edges.get(b).copied().unwrap_or(0.0)), short_num(edges.get(b + 1).copied().unwrap_or(0.0)));
            let vals = v
                .iter()
                .map(|x| {
                    if !x.is_finite() || edges.len() < 2 {
                        return Scalar::Null;
                    }
                    let nb = edges.len() - 1;
                    let b = edges.windows(2).position(|w| *x <= w.get(1).copied().unwrap_or(f64::INFINITY)).unwrap_or(nb - 1).min(nb - 1);
                    Scalar::Text(format!("{:02}. {}", b + 1, label(b)))
                })
                .collect();
            let name = s_str(node, "name");
            let name = if name.trim().is_empty() { format!("{col} bin") } else { name };
            let mut nt = (*t).clone();
            set_column(&mut nt, &name, vals, ColType::Text);
            Ok(out(nt))
        }
        "tf.sample" => {
            let t = in_table(inputs, 0)?;
            let n = t.rows.len();
            let mode = s_str(node, "mode");
            let take = match mode.as_str() {
                "fraction" => ((n as f64) * s_num(node, "fraction").clamp(0.0, 1.0)).round() as usize,
                _ => (s_num(node, "n").max(0.0) as usize).min(n),
            };
            let mut idx: Vec<usize> = (0..n).collect();
            if mode != "first" {
                ml::Rng::new(s_num(node, "seed") as u64).shuffle(&mut idx);
                let mut chosen: Vec<usize> = idx.get(..take).unwrap_or(&[]).to_vec();
                chosen.sort_unstable();
                let mut rest: Vec<usize> = idx.get(take..).unwrap_or(&[]).to_vec();
                rest.sort_unstable();
                idx = chosen.into_iter().chain(rest).collect();
            }
            let pick = |ids: &[usize]| TableData { rows: ids.iter().filter_map(|i| t.rows.get(*i).cloned()).collect(), ..(*t).clone() };
            let mut o = out(pick(idx.get(..take).unwrap_or(&[])));
            o.ports.push(PortData::Table(Rc::new(pick(idx.get(take..).unwrap_or(&[])))));
            Ok(o)
        }
        "tf.pivot" => {
            let t = in_table(inputs, 0)?;
            let rows_by = s_list(node, "rows");
            let pc = col_idx(&t, &s_str(node, "column"))?;
            let vc = col_idx(&t, &s_str(node, "value"))?;
            let op = s_str(node, "op");
            let rk: Vec<usize> = rows_by.iter().map(|c| col_idx(&t, c)).collect::<Result<_, _>>()?;
            let mut pivots: Vec<Scalar> = Vec::new();
            let mut seen = HashSet::new();
            for r in &t.rows {
                let v = r.get(pc).cloned().unwrap_or(Scalar::Null);
                if seen.insert(v.key()) {
                    pivots.push(v);
                }
            }
            pivots.sort_by(compare);
            if pivots.len() > 500 {
                return Err(format!("The pivot column has {} values: too many columns.", pivots.len()));
            }
            let mut groups: Vec<(Vec<Scalar>, Vec<Vec<f64>>, Vec<Vec<Scalar>>)> = Vec::new();
            let mut gi: HashMap<String, usize> = HashMap::new();
            for r in &t.rows {
                let key: Vec<Scalar> = rk.iter().map(|i| r.get(*i).cloned().unwrap_or(Scalar::Null)).collect();
                let ks = key.iter().map(Scalar::key).collect::<Vec<_>>().join("\u{1}");
                let g = *gi.entry(ks).or_insert_with(|| {
                    groups.push((key, vec![vec![]; pivots.len()], vec![vec![]; pivots.len()]));
                    groups.len() - 1
                });
                let pk = r.get(pc).map(Scalar::key).unwrap_or_default();
                if let Some(pi) = pivots.iter().position(|p| p.key() == pk)
                    && let Some(gr) = groups.get_mut(g)
                {
                    let v = r.get(vc).cloned().unwrap_or(Scalar::Null);
                    if let Some(x) = v.as_f64()
                        && let Some(s) = gr.1.get_mut(pi)
                    {
                        s.push(x);
                    }
                    if let Some(s) = gr.2.get_mut(pi) {
                        s.push(v);
                    }
                }
            }
            let mut columns = rows_by.clone();
            columns.extend(pivots.iter().map(|p| if p.is_null() { "(blank)".to_string() } else { p.text() }));
            let rows = groups.into_iter().map(|(key, nums, raw)| key.into_iter().chain(nums.iter().zip(&raw).map(|(n, r)| aggregate(&op, n, r))).collect()).collect();
            let mut nt = TableData { name: t.name.clone(), columns, types: vec![], rows };
            nt.infer_types();
            Ok(out(nt))
        }
        "tf.unpivot" => {
            let t = in_table(inputs, 0)?;
            let keep = s_list(node, "keep");
            let ki: Vec<usize> = keep.iter().map(|c| col_idx(&t, c)).collect::<Result<_, _>>()?;
            let others: Vec<usize> = (0..t.columns.len()).filter(|i| !ki.contains(i)).collect();
            let mut columns: Vec<String> = keep.clone();
            columns.push(s_str(node, "attribute"));
            columns.push(s_str(node, "value"));
            let mut rows = Vec::new();
            for r in &t.rows {
                for &o in &others {
                    let v = r.get(o).cloned().unwrap_or(Scalar::Null);
                    if v.is_null() {
                        continue;
                    }
                    let mut nr: Vec<Scalar> = ki.iter().map(|i| r.get(*i).cloned().unwrap_or(Scalar::Null)).collect();
                    nr.push(Scalar::Text(t.columns.get(o).cloned().unwrap_or_default()));
                    nr.push(v);
                    rows.push(nr);
                }
            }
            let mut nt = TableData { name: t.name.clone(), columns, types: vec![], rows };
            nt.infer_types();
            Ok(out(nt))
        }
        "tf.dedupe" => {
            let t = in_table(inputs, 0)?;
            let cols = s_list(node, "columns");
            let idx: Vec<usize> = if cols.is_empty() { (0..t.columns.len()).collect() } else { cols.iter().map(|c| col_idx(&t, c)).collect::<Result<_, _>>()? };
            let mut seen = HashSet::new();
            let rows: Vec<Vec<Scalar>> = t.rows.iter().filter(|r| seen.insert(idx.iter().map(|i| r.get(*i).map(Scalar::key).unwrap_or_default()).collect::<Vec<_>>().join("\u{1}"))).cloned().collect();
            let removed = t.rows.len() - rows.len();
            let mut o = out(TableData { rows, ..(*t).clone() });
            o.summary.metrics.push(("Duplicates removed".into(), fmt_int(removed)));
            Ok(o)
        }
        // ------------------------------------------------------------ unsupervised
        "ml.kmeans" | "ml.hclust" | "ml.dbscan" => {
            let t = in_table(inputs, 0)?;
            cluster_node(node, &t)
        }
        "ml.pca" => {
            let t = in_table(inputs, 0)?;
            let cols = feature_cols(node, &t, "columns", None);
            if cols.len() < 2 {
                return Err("PCA needs at least two numeric columns.".into());
            }
            let mut x = ml::matrix(&t, &cols)?;
            ml::impute_mean(&mut x);
            let k = (s_num(node, "components") as usize).clamp(1, cols.len());
            let r = ml::pca(&x, k, s_bool(node, "standardize"));
            let mut nt = if s_bool(node, "keep") { (*t).clone() } else { TableData { name: t.name.clone(), columns: vec![], types: vec![], rows: vec![vec![]; t.rows.len()] } };
            for c in 0..k {
                set_column(&mut nt, &format!("PC{}", c + 1), r.scores.iter().map(|s| Scalar::Num(s.get(c).copied().unwrap_or(0.0))).collect(), ColType::Number);
            }
            let mut cum = 0.0;
            let var_rows: Vec<Vec<Scalar>> = (0..k)
                .map(|c| {
                    let e = r.explained.get(c).copied().unwrap_or(0.0);
                    cum += e;
                    vec![Scalar::Text(format!("PC{}", c + 1)), Scalar::Num(r.variances.get(c).copied().unwrap_or(0.0)), Scalar::Num(e), Scalar::Num(cum)]
                })
                .collect();
            let var = TableData { name: "Explained variance".into(), columns: vec!["Component".into(), "Variance".into(), "Explained".into(), "Cumulative".into()], types: vec![ColType::Text, ColType::Number, ColType::Number, ColType::Number], rows: var_rows };
            let mut load_cols = vec!["Feature".to_string()];
            load_cols.extend((0..k).map(|c| format!("PC{}", c + 1)));
            let loadings = TableData { name: "Loadings".into(), columns: load_cols, types: vec![], rows: cols.iter().enumerate().map(|(i, c)| std::iter::once(Scalar::Text(c.clone())).chain((0..k).map(|j| Scalar::Num(r.loadings.get(i).and_then(|row| row.get(j)).copied().unwrap_or(0.0)))).collect()).collect() };
            let mut o = out(nt);
            o.ports.push(PortData::Table(Rc::new(var)));
            o.summary.metrics.push(("Components".into(), k.to_string()));
            o.summary.metrics.push(("Variance explained".into(), format!("{:.1}%", r.explained.iter().sum::<f64>() * 100.0)));
            o.summary.bars = Some(Bars { title: "Explained variance".into(), labels: (0..k).map(|c| format!("PC{}", c + 1)).collect(), values: r.explained.iter().map(|e| e * 100.0).collect() });
            if k >= 2 {
                o.summary.scatter = Some(Scatter { x_label: "PC1".into(), y_label: "PC2".into(), points: r.scores.iter().take(5000).map(|s| (s.first().copied().unwrap_or(0.0), s.get(1).copied().unwrap_or(0.0), 0)).collect(), groups: vec!["Rows".into()], centers: vec![] });
            }
            o.summary.tables.push(("Loadings".into(), Rc::new(loadings)));
            Ok(o)
        }
        // ------------------------------------------------------------ supervised
        "ml.split" => {
            let t = in_table(inputs, 0)?;
            let share = s_num(node, "train").clamp(0.0, 1.0);
            let mut rng = ml::Rng::new(s_num(node, "seed") as u64);
            let strat = s_str(node, "stratify");
            let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
            if strat.is_empty() {
                groups.insert(String::new(), (0..t.rows.len()).collect());
            } else {
                let si = col_idx(&t, &strat)?;
                for (i, r) in t.rows.iter().enumerate() {
                    groups.entry(r.get(si).map(Scalar::key).unwrap_or_default()).or_default().push(i);
                }
            }
            let (mut train, mut test) = (Vec::new(), Vec::new());
            for (_, mut g) in groups {
                rng.shuffle(&mut g);
                let k = (g.len() as f64 * share).round() as usize;
                train.extend(g.get(..k).unwrap_or(&[]).iter().copied());
                test.extend(g.get(k..).unwrap_or(&[]).iter().copied());
            }
            train.sort_unstable();
            test.sort_unstable();
            let pick = |ids: &[usize]| TableData { rows: ids.iter().filter_map(|i| t.rows.get(*i).cloned()).collect(), ..(*t).clone() };
            let mut o = out(pick(&train));
            o.ports.push(PortData::Table(Rc::new(pick(&test))));
            o.summary.metrics.push(("Training rows".into(), fmt_int(train.len())));
            o.summary.metrics.push(("Test rows".into(), fmt_int(test.len())));
            Ok(o)
        }
        "ml.linreg" | "ml.logreg" | "ml.tree" | "ml.forest" | "ml.nb" | "ml.knn" => {
            let t = in_table(inputs, 0)?;
            learner_node(node, &t)
        }
        "ml.predict" => {
            let Some(Some(PortData::Model(m))) = inputs.first() else { return Err("Connect a trained model (a learner's model output).".into()) };
            let t = in_table(inputs, 1)?;
            let (nt, _, _) = predict_table(m, &t, &s_str(node, "output"), s_bool(node, "probabilities"))?;
            Ok(out(nt))
        }
        "ml.score" => {
            let Some(Some(PortData::Model(m))) = inputs.first() else { return Err("Connect a trained model (a learner's model output).".into()) };
            let t = in_table(inputs, 1)?;
            score_node(m, &t)
        }
        "ml.confusion" => {
            let t = in_table(inputs, 0)?;
            let a = col_idx(&t, &s_str(node, "actual"))?;
            let p = col_idx(&t, &s_str(node, "predicted"))?;
            let mut labels: Vec<String> = t.rows.iter().flat_map(|r| [r.get(a), r.get(p)]).flatten().filter(|v| !v.is_null()).map(Scalar::text).collect();
            labels.sort();
            labels.dedup();
            if labels.len() > 60 {
                return Err("Too many classes for a confusion matrix.".into());
            }
            let pos = |v: Option<&Scalar>| v.filter(|x| !x.is_null()).and_then(|x| labels.iter().position(|l| *l == x.text()));
            let pairs: Vec<(usize, usize)> = t.rows.iter().filter_map(|r| Some((pos(r.get(a))?, pos(r.get(p))?))).collect();
            let cm = ml::class_metrics(&pairs.iter().map(|x| x.0).collect::<Vec<_>>(), &pairs.iter().map(|x| x.1).collect::<Vec<_>>(), labels.len(), None);
            let mut o = out(confusion_table(&labels, &cm.confusion));
            o.summary.metrics = class_metric_rows(&cm);
            o.summary.heatmap = Some((labels.clone(), labels, cm.confusion.iter().map(|r| r.iter().map(|v| *v as f64).collect()).collect()));
            Ok(o)
        }
        // ------------------------------------------------------------ statistics
        "stat.outliers" => {
            let t = in_table(inputs, 0)?;
            let cols = feature_cols(node, &t, "columns", None);
            if cols.is_empty() {
                return Err("Choose numeric columns.".into());
            }
            let x = ml::matrix(&t, &cols)?;
            let method = s_str(node, "method");
            let r = ml::outliers(&x, &method, s_num(node, "threshold"), 42);
            let action = s_str(node, "action");
            let mut nt = (*t).clone();
            set_column(&mut nt, "Outlier score", r.iter().map(|v| Scalar::Num((v.0 * 1000.0).round() / 1000.0)).collect(), ColType::Number);
            set_column(&mut nt, "Outlier", r.iter().map(|v| Scalar::Bool(v.1)).collect(), ColType::Bool);
            let count = r.iter().filter(|v| v.1).count();
            if action != "flag" {
                let keep_outliers = action == "only";
                nt.rows = nt.rows.into_iter().zip(&r).filter(|(_, v)| v.1 == keep_outliers).map(|(row, _)| row).collect();
            }
            let mut o = out(nt);
            o.summary.metrics.push(("Outliers".into(), format!("{} of {} ({:.1}%)", fmt_int(count), fmt_int(r.len()), count as f64 * 100.0 / r.len().max(1) as f64)));
            if cols.len() >= 2 {
                o.summary.scatter = Some(Scatter { x_label: cols.first().cloned().unwrap_or_default(), y_label: cols.get(1).cloned().unwrap_or_default(), points: x.iter().zip(&r).take(5000).map(|(p, v)| (p.first().copied().unwrap_or(0.0), p.get(1).copied().unwrap_or(0.0), usize::from(v.1))).collect(), groups: vec!["Normal".into(), "Outlier".into()], centers: vec![] });
            }
            Ok(o)
        }
        "stat.corr" => {
            let t = in_table(inputs, 0)?;
            let cols = feature_cols(node, &t, "columns", None);
            if cols.len() < 2 {
                return Err("Correlations need at least two numeric columns.".into());
            }
            let x = ml::matrix(&t, &cols)?;
            let colv: Vec<Vec<f64>> = (0..cols.len()).map(|j| x.iter().map(|r| r.get(j).copied().unwrap_or(f64::NAN)).collect()).collect();
            let m: Vec<Vec<f64>> = (0..cols.len()).map(|i| (0..cols.len()).map(|j| ml::pearson(colv.get(i).map(Vec::as_slice).unwrap_or(&[]), colv.get(j).map(Vec::as_slice).unwrap_or(&[]))).collect()).collect();
            let mut columns = vec!["Feature".to_string()];
            columns.extend(cols.iter().cloned());
            let rows = cols.iter().zip(&m).map(|(c, r)| std::iter::once(Scalar::Text(c.clone())).chain(r.iter().map(|v| if v.is_finite() { Scalar::Num((v * 10000.0).round() / 10000.0) } else { Scalar::Null })).collect()).collect();
            let mut nt = TableData { name: "Correlations".into(), columns, types: vec![], rows };
            nt.infer_types();
            let mut pairs: Vec<(String, String, f64)> = Vec::new();
            for i in 0..cols.len() {
                for j in (i + 1)..cols.len() {
                    let v = m.get(i).and_then(|r| r.get(j)).copied().unwrap_or(f64::NAN);
                    if v.is_finite() {
                        pairs.push((cols.get(i).cloned().unwrap_or_default(), cols.get(j).cloned().unwrap_or_default(), v));
                    }
                }
            }
            pairs.sort_by(|a, b| b.2.abs().partial_cmp(&a.2.abs()).unwrap_or(std::cmp::Ordering::Equal));
            let top = TableData { name: "Strongest pairs".into(), columns: vec!["Feature A".into(), "Feature B".into(), "Correlation".into()], types: vec![ColType::Text, ColType::Text, ColType::Number], rows: pairs.iter().take(20).map(|(a, b, v)| vec![Scalar::Text(a.clone()), Scalar::Text(b.clone()), Scalar::Num((v * 10000.0).round() / 10000.0)]).collect() };
            let mut o = out(nt);
            o.summary.heatmap = Some((cols.clone(), cols, m));
            o.summary.tables.push(("Strongest pairs".into(), Rc::new(top)));
            Ok(o)
        }
        "stat.stats" => {
            let t = in_table(inputs, 0)?;
            let mut o = out(ml::feature_stats(&t));
            o.summary.metrics.push(("Rows".into(), fmt_int(t.rows.len())));
            o.summary.metrics.push(("Columns".into(), t.columns.len().to_string()));
            Ok(o)
        }
        "stat.dist" => {
            let t = in_table(inputs, 0)?;
            let col = s_str(node, "column");
            let col = if col.is_empty() { ml::numeric_columns(&t).first().cloned().or_else(|| t.columns.first().cloned()).unwrap_or_default() } else { col };
            let i = col_idx(&t, &col)?;
            let ty = t.types.get(i).copied().unwrap_or_default();
            let (labels, values): (Vec<String>, Vec<f64>) = if ty.is_numeric() || (ty == ColType::Any && t.rows.iter().any(|r| matches!(r.get(i), Some(Scalar::Num(_))))) {
                let v: Vec<f64> = t.rows.iter().filter_map(|r| r.get(i).and_then(Scalar::as_f64)).collect();
                ml::histogram(&v, s_num(node, "bins") as usize).into_iter().map(|(a, b, c)| (format!("{} – {}", short_num(a), short_num(b)), c as f64)).unzip()
            } else {
                let mut counts: BTreeMap<String, usize> = BTreeMap::new();
                for r in &t.rows {
                    *counts.entry(r.get(i).map(|v| if v.is_null() { "(blank)".to_string() } else { v.text() }).unwrap_or_default()).or_default() += 1;
                }
                let mut v: Vec<(String, usize)> = counts.into_iter().collect();
                v.sort_by(|a, b| b.1.cmp(&a.1));
                v.truncate(60);
                v.into_iter().map(|(a, b)| (a, b as f64)).unzip()
            };
            let total: f64 = values.iter().sum();
            let nt = TableData {
                name: format!("{col} distribution"),
                columns: vec![col.clone(), "Count".into(), "Share".into()],
                types: vec![ColType::Text, ColType::Integer, ColType::Number],
                rows: labels.iter().zip(&values).map(|(l, v)| vec![Scalar::Text(l.clone()), Scalar::Num(*v), Scalar::Num(if total > 0.0 { v / total } else { 0.0 })]).collect(),
            };
            let mut o = out(nt);
            o.summary.bars = Some(Bars { title: col, labels, values });
            Ok(o)
        }
        // ------------------------------------------------------------ scripting
        "script.python" => {
            let mut ins: Vec<(String, TableData)> = Vec::new();
            for (i, name) in ["in_df", "in_df2"].iter().enumerate() {
                if let Some(Some(PortData::Table(t))) = inputs.get(i) {
                    ins.push((name.to_string(), (**t).clone()));
                }
            }
            let r = run_python(&PyJob { prelude: String::new(), code: s_str(node, "code"), inputs: ins, want_out_df: true, timeout_s: 120 })?;
            let mut o = NodeOutput::default();
            o.summary.log = r.log();
            match r.out_df {
                Some(t) => o.ports.push(PortData::Table(Rc::new(t))),
                None => return Err(if r.stderr.trim().is_empty() { "The script didn't set out_df.".into() } else { r.stderr.trim().to_string() }),
            }
            if !r.ok {
                return Err(r.stderr.trim().to_string());
            }
            Ok(o)
        }
        // ------------------------------------------------------------ outputs
        "out.table" => {
            let t = in_table(inputs, 0)?;
            let mut o = NodeOutput { ports: vec![PortData::Table(t.clone())], ..NodeOutput::default() };
            o.summary.metrics.push(("Rows".into(), fmt_int(t.rows.len())));
            o.summary.metrics.push(("Columns".into(), t.columns.len().to_string()));
            Ok(o)
        }
        "out.csv" => {
            let t = in_table(inputs, 0)?;
            let path = s_str(node, "path");
            let mut o = NodeOutput { ports: vec![PortData::Table(t.clone())], ..NodeOutput::default() };
            if !path.is_empty() {
                std::fs::write(&path, write_csv(&t, ',')).map_err(|e| format!("Couldn't write {path}: {e}"))?;
                o.summary.metrics.push(("Written".into(), path));
            }
            Ok(o)
        }
        other => Err(format!("Unknown node type `{other}`.")),
    }
}

// =============================================================================================
// Node helpers
// =============================================================================================

fn fmt_int(n: usize) -> String {
    crate::numfmt::format(n as f64, "#,0")
}

fn short_num(x: f64) -> String {
    if x.abs() >= 1000.0 { crate::numfmt::format(x, "#,0") } else if x.abs() >= 10.0 { crate::numfmt::format(x, "0.#") } else { crate::numfmt::format(x, "0.##") }
}

fn project_cols(t: &TableData, idx: &[usize]) -> TableData {
    TableData {
        name: t.name.clone(),
        columns: idx.iter().filter_map(|i| t.columns.get(*i).cloned()).collect(),
        types: idx.iter().map(|i| t.types.get(*i).copied().unwrap_or_default()).collect(),
        rows: t.rows.iter().map(|r| idx.iter().map(|i| r.get(*i).cloned().unwrap_or(Scalar::Null)).collect()).collect(),
    }
}

/// Adds (or replaces) a column.
pub fn set_column(t: &mut TableData, name: &str, vals: Vec<Scalar>, ty: ColType) {
    let ty = if ty == ColType::Any {
        let mut tmp = TableData { columns: vec![name.into()], rows: vals.iter().take(5000).map(|v| vec![v.clone()]).collect(), ..TableData::default() };
        tmp.infer_types();
        tmp.types.first().copied().unwrap_or_default()
    } else {
        ty
    };
    match t.column_index(name) {
        Some(i) => {
            for (r, v) in t.rows.iter_mut().zip(vals) {
                if let Some(c) = r.get_mut(i) {
                    *c = v;
                }
            }
            if let Some(x) = t.types.get_mut(i) {
                *x = ty;
            }
        }
        None => {
            t.columns.push(name.to_string());
            t.types.resize(t.columns.len() - 1, ColType::Any);
            t.types.push(ty);
            let w = t.columns.len() - 1;
            for (r, v) in t.rows.iter_mut().zip(vals) {
                r.resize(w, Scalar::Null);
                r.push(v);
            }
        }
    }
}

fn condition_value(v: &str, ty: ColType) -> Scalar {
    if ty.is_temporal()
        && let Some(d) = crate::build::parse_date(v)
    {
        return Scalar::Num(d);
    }
    if ty.is_numeric() || ty == ColType::Any {
        let p = crate::sheet::parse_input(v);
        if matches!(p, Scalar::Num(_)) {
            return p;
        }
    }
    if v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("false") {
        return Scalar::Bool(v.eq_ignore_ascii_case("true"));
    }
    Scalar::Text(v.to_string())
}

fn test(v: &Scalar, op: &str, target: &Scalar) -> bool {
    use std::cmp::Ordering::*;
    let lower = |s: &Scalar| if s.is_null() { String::new() } else { s.text().to_lowercase() };
    match op {
        "blank" => v.is_null() || matches!(v, Scalar::Text(t) if t.trim().is_empty()),
        "not_blank" => !(v.is_null() || matches!(v, Scalar::Text(t) if t.trim().is_empty())),
        "contains" => lower(v).contains(&lower(target)),
        "not_contains" => !lower(v).contains(&lower(target)),
        "starts" => lower(v).starts_with(&lower(target)),
        "ends" => lower(v).ends_with(&lower(target)),
        _ => {
            if v.is_null() {
                return op == "<>";
            }
            let o = match (v, target) {
                (Scalar::Num(a), Scalar::Num(b)) => a.partial_cmp(b).unwrap_or(Equal),
                (Scalar::Text(a), Scalar::Num(b)) => match a.trim().parse::<f64>() {
                    Ok(x) => x.partial_cmp(b).unwrap_or(Equal),
                    Err(_) => a.to_lowercase().cmp(&crate::numfmt::general(*b)),
                },
                (Scalar::Num(a), Scalar::Text(b)) => crate::numfmt::general(*a).cmp(&b.to_lowercase()),
                _ => compare(v, target),
            };
            match op {
                "=" => o == Equal,
                "<>" => o != Equal,
                ">" => o == Greater,
                ">=" => o != Less,
                "<" => o == Less,
                "<=" => o != Greater,
                _ => false,
            }
        }
    }
}

fn aggregate(op: &str, nums: &[f64], raw: &[Scalar]) -> Scalar {
    let non_null: Vec<&Scalar> = raw.iter().filter(|v| !v.is_null()).collect();
    match op {
        "count" => Scalar::Num(raw.len() as f64),
        "count_distinct" => Scalar::Num(non_null.iter().map(|v| v.key()).collect::<HashSet<_>>().len() as f64),
        "first" => non_null.first().map(|v| (*v).clone()).unwrap_or(Scalar::Null),
        "last" => non_null.last().map(|v| (*v).clone()).unwrap_or(Scalar::Null),
        _ if nums.is_empty() => Scalar::Null,
        "sum" => Scalar::Num(nums.iter().sum()),
        "mean" => Scalar::Num(ml::mean(nums)),
        "median" => Scalar::Num(ml::quantile_sorted(&ml::sorted_finite(nums), 0.5)),
        "min" => Scalar::Num(nums.iter().copied().fold(f64::INFINITY, f64::min)),
        "max" => Scalar::Num(nums.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
        "std" => Scalar::Num(ml::std_dev(nums)),
        _ => Scalar::Null,
    }
}

/// Group by `by` with aggregations `(column, op, output name)`.
pub fn group_by(t: &TableData, by: &[String], aggs: &[(String, String, String)]) -> Result<TableData, String> {
    let bi: Vec<usize> = by.iter().map(|c| col_idx(t, c)).collect::<Result<_, _>>()?;
    let ai: Vec<Option<usize>> = aggs.iter().map(|(c, op, _)| if c.is_empty() && op == "count" { Ok(None) } else { col_idx(t, c).map(Some) }).collect::<Result<_, _>>()?;
    let mut order: Vec<Vec<Scalar>> = Vec::new();
    let mut groups: HashMap<String, (usize, Vec<Vec<f64>>, Vec<Vec<Scalar>>)> = HashMap::new();
    for r in &t.rows {
        let key: Vec<Scalar> = bi.iter().map(|i| r.get(*i).cloned().unwrap_or(Scalar::Null)).collect();
        let ks = key.iter().map(Scalar::key).collect::<Vec<_>>().join("\u{1}");
        let n_aggs = aggs.len();
        let g = groups.entry(ks).or_insert_with(|| {
            order.push(key.clone());
            (order.len() - 1, vec![vec![]; n_aggs], vec![vec![]; n_aggs])
        });
        for (a, ci) in ai.iter().enumerate() {
            let v = ci.and_then(|i| r.get(i)).cloned().unwrap_or(Scalar::Bool(true));
            if let Some(x) = v.as_f64().filter(|_| !matches!(v, Scalar::Bool(_)) || ci.is_none())
                && let Some(s) = g.1.get_mut(a)
            {
                s.push(x);
            }
            if let Some(s) = g.2.get_mut(a) {
                s.push(v);
            }
        }
    }
    let mut sorted: Vec<(usize, Vec<Vec<f64>>, Vec<Vec<Scalar>>)> = groups.into_values().collect();
    sorted.sort_by_key(|g| g.0);
    let mut columns: Vec<String> = by.to_vec();
    for (c, op, name) in aggs {
        columns.push(if name.trim().is_empty() {
            let label = AGG_OPS.iter().find(|(k, _)| k == op).map(|x| x.1).unwrap_or(op.as_str());
            if c.is_empty() { label.to_string() } else { format!("{label} of {c}") }
        } else {
            name.clone()
        });
    }
    let rows = sorted.into_iter().map(|(gi, nums, raw)| order.get(gi).cloned().unwrap_or_default().into_iter().chain(aggs.iter().enumerate().map(|(a, (_, op, _))| aggregate(op, nums.get(a).map(Vec::as_slice).unwrap_or(&[]), raw.get(a).map(Vec::as_slice).unwrap_or(&[])))).collect()).collect();
    let mut types: Vec<ColType> = bi.iter().map(|i| t.types.get(*i).copied().unwrap_or_default()).collect();
    types.extend(aggs.iter().map(|_| ColType::Any));
    let mut out = TableData { name: t.name.clone(), columns, types, rows };
    out.infer_types();
    Ok(out)
}

/// Joins two tables on keys (`inner`, `left`, `right`, `full`).
pub fn join(l: &TableData, r: &TableData, lk: &[String], rk: &[String], kind: &str) -> Result<TableData, String> {
    let li: Vec<usize> = lk.iter().map(|c| col_idx(l, c)).collect::<Result<_, _>>()?;
    let ri: Vec<usize> = rk.iter().map(|c| col_idx(r, c)).collect::<Result<_, _>>()?;
    // Right columns other than keys that share the left key's name.
    let r_keep: Vec<usize> = (0..r.columns.len()).filter(|i| !(ri.contains(i) && lk.iter().zip(rk).any(|(a, b)| a.eq_ignore_ascii_case(b) && r.columns.get(*i).is_some_and(|c| c.eq_ignore_ascii_case(b))))).collect();
    let mut columns = l.columns.clone();
    let mut types = l.types.clone();
    types.resize(columns.len(), ColType::Any);
    for &i in &r_keep {
        let mut name = r.columns.get(i).cloned().unwrap_or_default();
        if columns.iter().any(|c| c.eq_ignore_ascii_case(&name)) {
            name = format!("{name} (right)");
        }
        columns.push(name);
        types.push(r.types.get(i).copied().unwrap_or_default());
    }
    let key = |row: &Vec<Scalar>, idx: &[usize]| -> Option<String> {
        let parts: Vec<String> = idx.iter().map(|i| row.get(*i).cloned().unwrap_or(Scalar::Null)).map(|v| if v.is_null() { None } else { Some(v.key()) }).collect::<Option<Vec<_>>>()?;
        Some(parts.join("\u{1}"))
    };
    let mut index: HashMap<String, Vec<usize>> = HashMap::new();
    for (j, row) in r.rows.iter().enumerate() {
        if let Some(k) = key(row, &ri) {
            index.entry(k).or_default().push(j);
        }
    }
    let lw = l.columns.len();
    let mut used = vec![false; r.rows.len()];
    let mut rows = Vec::new();
    let keep_left = kind == "left" || kind == "full";
    let keep_right = kind == "right" || kind == "full";
    for lrow in &l.rows {
        let matches = key(lrow, &li).and_then(|k| index.get(&k));
        match matches {
            Some(js) => {
                for &j in js {
                    if let Some(u) = used.get_mut(j) {
                        *u = true;
                    }
                    let rrow = r.rows.get(j);
                    let mut row: Vec<Scalar> = lrow.clone();
                    row.resize(lw, Scalar::Null);
                    row.extend(r_keep.iter().map(|i| rrow.and_then(|x| x.get(*i)).cloned().unwrap_or(Scalar::Null)));
                    rows.push(row);
                }
            }
            None if keep_left => {
                let mut row = lrow.clone();
                row.resize(lw, Scalar::Null);
                row.extend(r_keep.iter().map(|_| Scalar::Null));
                rows.push(row);
            }
            None => {}
        }
    }
    if keep_right {
        for (j, rrow) in r.rows.iter().enumerate() {
            if used.get(j).copied().unwrap_or(true) {
                continue;
            }
            let mut row = vec![Scalar::Null; lw];
            // Fill left key columns from the right keys.
            for (a, b) in li.iter().zip(&ri) {
                if let Some(c) = row.get_mut(*a) {
                    *c = rrow.get(*b).cloned().unwrap_or(Scalar::Null);
                }
            }
            row.extend(r_keep.iter().map(|i| rrow.get(*i).cloned().unwrap_or(Scalar::Null)));
            rows.push(row);
        }
    }
    Ok(TableData { name: l.name.clone(), columns, types, rows })
}

/// Feature columns from a setting (empty: numeric columns, minus `exclude`).
fn feature_cols(node: &FlowNode, t: &TableData, key: &str, exclude: Option<&str>) -> Vec<String> {
    let c = s_list(node, key);
    if !c.is_empty() {
        return c;
    }
    ml::numeric_columns(t).into_iter().filter(|c| exclude.is_none_or(|e| !c.eq_ignore_ascii_case(e))).collect()
}

fn cluster_node(node: &FlowNode, t: &TableData) -> Result<NodeOutput, String> {
    let cols = feature_cols(node, t, "columns", None);
    if cols.is_empty() {
        return Err("Choose numeric columns to cluster on.".into());
    }
    let raw = ml::matrix(t, &cols)?;
    let mut x = raw.clone();
    ml::impute_mean(&mut x);
    let norm = s_bool(node, "normalize");
    let scaler = ml::Scaler::fit(&x);
    let z = if norm { scaler.apply(&x) } else { x.clone() };
    let mut summary = Summary::default();
    let mut extra: Option<TableData> = None;
    let labels: Vec<Option<usize>> = match node.kind.as_str() {
        "ml.kmeans" => {
            let seed = s_num(node, "seed") as u64;
            let (r, k) = if s_bool(node, "auto") {
                let (r, k, scores) = ml::kmeans_auto(&z, (s_num(node, "max_k") as usize).max(2), seed);
                summary.tables.push((
                    "Silhouette by k".into(),
                    Rc::new(TableData { name: "Silhouette".into(), columns: vec!["k".into(), "Silhouette".into()], types: vec![ColType::Integer, ColType::Number], rows: scores.iter().map(|(k, s)| vec![Scalar::Num(*k as f64), Scalar::Num((s * 10000.0).round() / 10000.0)]).collect() }),
                ));
                (r, k)
            } else {
                let k = (s_num(node, "k") as usize).max(1);
                (ml::kmeans(&z, k, seed, 300), k)
            };
            summary.metrics.push(("Clusters (k)".into(), k.to_string()));
            summary.metrics.push(("Inertia".into(), short_num(r.inertia)));
            summary.metrics.push(("Iterations".into(), r.iterations.to_string()));
            // Centroids in original units.
            let mut ccols = vec!["Cluster".to_string()];
            ccols.extend(cols.iter().cloned());
            let crow = r
                .centroids
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    std::iter::once(Scalar::Text(format!("C{}", i + 1)))
                        .chain(c.iter().enumerate().map(|(j, v)| Scalar::Num(if norm { v * scaler.std.get(j).copied().unwrap_or(1.0) + scaler.mean.get(j).copied().unwrap_or(0.0) } else { *v })))
                        .collect()
                })
                .collect();
            let mut ct = TableData { name: "Centroids".into(), columns: ccols, types: vec![], rows: crow };
            ct.infer_types();
            extra = Some(ct);
            r.labels.into_iter().map(Some).collect()
        }
        "ml.hclust" => ml::hierarchical(&z, (s_num(node, "k") as usize).max(1))?.into_iter().map(Some).collect(),
        _ => {
            if z.len() > 20_000 {
                return Err("DBSCAN handles up to 20,000 rows: add a Sample node first.".into());
            }
            ml::dbscan(&z, s_num(node, "eps"), (s_num(node, "min_points") as usize).max(1))
        }
    };
    let k = labels.iter().flatten().copied().max().map(|m| m + 1).unwrap_or(0);
    let mut groups: Vec<String> = (0..k).map(|i| format!("C{}", i + 1)).collect();
    let has_noise = labels.iter().any(Option::is_none);
    if has_noise {
        groups.push("Noise".into());
    }
    let gid = |l: &Option<usize>| l.unwrap_or(k);
    let dense: Vec<usize> = labels.iter().map(|l| l.unwrap_or(0)).collect();
    if k >= 2 && labels.iter().all(Option::is_some) {
        summary.metrics.push(("Silhouette".into(), format!("{:.3}", ml::silhouette(&z, &dense))));
    }
    if has_noise {
        summary.metrics.push(("Noise rows".into(), fmt_int(labels.iter().filter(|l| l.is_none()).count())));
    }
    let mut sizes = vec![0.0; groups.len()];
    for l in &labels {
        if let Some(s) = sizes.get_mut(gid(l)) {
            *s += 1.0;
        }
    }
    summary.bars = Some(Bars { title: "Cluster sizes".into(), labels: groups.clone(), values: sizes });
    // Scatter on the first two features (original units).
    let (xi, yi) = (0, if cols.len() > 1 { 1 } else { 0 });
    let points: Vec<(f64, f64, usize)> = raw.iter().zip(&labels).take(5000).map(|(r, l)| (r.get(xi).copied().unwrap_or(f64::NAN), r.get(yi).copied().unwrap_or(f64::NAN), gid(l))).filter(|p| p.0.is_finite() && p.1.is_finite()).collect();
    let centers = extra.as_ref().map(|c| c.rows.iter().map(|r| (r.get(1 + xi).and_then(Scalar::as_f64).unwrap_or(0.0), r.get(1 + yi).and_then(Scalar::as_f64).unwrap_or(0.0))).collect()).unwrap_or_default();
    summary.scatter = Some(Scatter { x_label: cols.get(xi).cloned().unwrap_or_default(), y_label: cols.get(yi).cloned().unwrap_or_default(), points, groups: groups.clone(), centers });
    let name = s_str(node, "output");
    let name = if name.trim().is_empty() { "Cluster".to_string() } else { name };
    let mut nt = t.clone();
    set_column(&mut nt, &name, labels.iter().map(|l| Scalar::Text(groups.get(gid(l)).cloned().unwrap_or_default())).collect(), ColType::Text);
    let mut o = out(nt);
    if let Some(e) = extra {
        o.ports.push(PortData::Table(Rc::new(e)));
    }
    o.summary = summary;
    Ok(o)
}

fn learner_node(node: &FlowNode, t: &TableData) -> Result<NodeOutput, String> {
    let target = s_str(node, "target");
    if target.is_empty() {
        return Err("Choose the target column to predict.".into());
    }
    let features = feature_cols(node, t, "features", Some(&target));
    let kind = match node.kind.as_str() {
        "ml.linreg" => "linear",
        "ml.logreg" => "logistic",
        "ml.tree" => "tree",
        "ml.forest" => "forest",
        "ml.nb" => "naive_bayes",
        _ => "knn",
    };
    let mut p = ml::TrainParams { kind: kind.into(), ..ml::TrainParams::default() };
    if node.config.contains_key("task") || node_type(&node.kind).is_some_and(|t| t.param("task").is_some()) {
        p.task = s_str(node, "task");
    }
    if kind == "logistic" {
        p.iterations = s_num(node, "iterations").max(1.0) as usize;
        p.learning_rate = s_num(node, "learning_rate").max(1e-6);
        p.l2 = s_num(node, "l2").max(0.0);
    }
    if kind == "tree" || kind == "forest" {
        p.tree.max_depth = s_num(node, "max_depth").max(1.0) as usize;
    }
    if kind == "tree" {
        p.tree.min_samples_leaf = s_num(node, "min_leaf").max(1.0) as usize;
        p.tree.min_samples_split = p.tree.min_samples_leaf * 2;
    }
    if kind == "forest" {
        p.trees = s_num(node, "trees").max(1.0) as usize;
        p.seed = s_num(node, "seed") as u64;
    }
    if kind == "knn" {
        p.k = s_num(node, "k").max(1.0) as usize;
    }
    let m = ml::train(t, &features, &target, &p)?;
    let (fitted, _, _) = predict_table(&m, t, "Prediction", false)?;
    let mut o = NodeOutput::default();
    o.summary.metrics = m.info.clone();
    o.summary.metrics.insert(0, ("Task".into(), if m.is_classifier() { format!("Classification ({} classes)", m.classes.len()) } else { "Regression".into() }));
    o.summary.metrics.insert(1, ("Features".into(), m.features.join(", ")));
    if let ml::Learner::Linear { coef, std_err } = &m.learner {
        let rows = std::iter::once("(Intercept)".to_string())
            .chain(m.features.iter().cloned())
            .enumerate()
            .map(|(i, name)| {
                let c = coef.get(i).copied().unwrap_or(0.0);
                let se = std_err.get(i).copied().unwrap_or(0.0);
                vec![Scalar::Text(name), Scalar::Num(c), Scalar::Num(se), if se > 0.0 { Scalar::Num(c / se) } else { Scalar::Null }]
            })
            .collect();
        o.summary.tables.push(("Coefficients".into(), Rc::new(TableData { name: "Coefficients".into(), columns: vec!["Term".into(), "Coefficient".into(), "Std. error".into(), "t".into()], types: vec![ColType::Text, ColType::Number, ColType::Number, ColType::Number], rows })));
    }
    if !m.is_classifier()
        && let (Some(ti), Some(pi)) = (fitted.column_index(&m.target), fitted.column_index("Prediction"))
    {
        o.summary.scatter = Some(Scatter { x_label: format!("Actual {}", m.target), y_label: "Predicted".into(), points: fitted.rows.iter().take(5000).filter_map(|r| Some((r.get(ti)?.as_f64()?, r.get(pi)?.as_f64()?, 0))).collect(), groups: vec!["Rows".into()], centers: vec![] });
    }
    o.ports.push(PortData::Model(Rc::new(m)));
    o.ports.push(PortData::Table(Rc::new(fitted)));
    Ok(o)
}

/// Adds predictions (and class probabilities) to a table. Returns the table, the predictions
/// and the probabilities.
pub fn predict_table(m: &ml::Model, t: &TableData, out_name: &str, probs: bool) -> Result<(TableData, Vec<f64>, Vec<Vec<f64>>), String> {
    let x = ml::matrix(t, &m.features).map_err(|e| format!("{e} (the model was trained on {}).", m.features.join(", ")))?;
    let preds: Vec<(f64, Vec<f64>)> = x.iter().map(|r| m.predict_row(r)).collect();
    let mut nt = t.clone();
    let name = if out_name.trim().is_empty() { "Prediction" } else { out_name };
    if m.is_classifier() {
        let numeric_classes = m.classes.iter().all(|c| c.parse::<f64>().is_ok()) && t.column_index(&m.target).is_some_and(|i| t.types.get(i).copied().unwrap_or_default().is_numeric());
        set_column(
            &mut nt,
            name,
            preds
                .iter()
                .map(|p| {
                    let c = m.classes.get(p.0 as usize).cloned().unwrap_or_default();
                    if numeric_classes { c.parse::<f64>().map(Scalar::Num).unwrap_or(Scalar::Text(c)) } else { Scalar::Text(c) }
                })
                .collect(),
            ColType::Any,
        );
        if probs {
            for (ci, c) in m.classes.iter().enumerate() {
                set_column(&mut nt, &format!("P({c})"), preds.iter().map(|p| Scalar::Num((p.1.get(ci).copied().unwrap_or(0.0) * 10000.0).round() / 10000.0)).collect(), ColType::Number);
            }
        }
    } else {
        set_column(&mut nt, name, preds.iter().map(|p| Scalar::Num(p.0)).collect(), ColType::Number);
    }
    Ok((nt, preds.iter().map(|p| p.0).collect(), preds.into_iter().map(|p| p.1).collect()))
}

fn score_node(m: &ml::Model, t: &TableData) -> Result<NodeOutput, String> {
    let ti = t.column_index(&m.target).ok_or_else(|| format!("The test data has no `{}` column (the target).", m.target))?;
    let (pt, preds, probs) = predict_table(m, t, "Prediction", m.is_classifier())?;
    let mut o = NodeOutput::default();
    let metrics: Vec<(String, String)>;
    if m.is_classifier() {
        let mut actual = Vec::new();
        let mut pred = Vec::new();
        let mut pr = Vec::new();
        for (i, r) in t.rows.iter().enumerate() {
            let a = r.get(ti).filter(|v| !v.is_null()).map(Scalar::text);
            if let Some(ci) = a.and_then(|a| m.classes.iter().position(|c| *c == a)) {
                actual.push(ci);
                pred.push(preds.get(i).copied().unwrap_or(0.0) as usize);
                pr.push(probs.get(i).cloned().unwrap_or_default());
            }
        }
        let cm = ml::class_metrics(&actual, &pred, m.classes.len(), Some(&pr));
        metrics = class_metric_rows(&cm);
        o.summary.tables.push(("Confusion matrix".into(), Rc::new(confusion_table(&m.classes, &cm.confusion))));
        o.summary.heatmap = Some((m.classes.clone(), m.classes.clone(), cm.confusion.iter().map(|r| r.iter().map(|v| *v as f64).collect()).collect()));
    } else {
        let mut a = Vec::new();
        let mut p = Vec::new();
        for (i, r) in t.rows.iter().enumerate() {
            if let Some(v) = r.get(ti).and_then(Scalar::as_f64) {
                a.push(v);
                p.push(preds.get(i).copied().unwrap_or(0.0));
            }
        }
        let rm = ml::regression_metrics(&a, &p);
        metrics = vec![("RMSE".into(), short_num(rm.rmse)), ("MAE".into(), short_num(rm.mae)), ("R²".into(), format!("{:.4}", rm.r2)), ("Test rows".into(), fmt_int(rm.n))];
        o.summary.scatter = Some(Scatter { x_label: format!("Actual {}", m.target), y_label: "Predicted".into(), points: a.iter().zip(&p).take(5000).map(|(x, y)| (*x, *y, 0)).collect(), groups: vec!["Test rows".into()], centers: vec![] });
    }
    let mt = TableData { name: "Scores".into(), columns: vec!["Model".into(), "Metric".into(), "Value".into()], types: vec![ColType::Text, ColType::Text, ColType::Text], rows: metrics.iter().map(|(k, v)| vec![Scalar::Text(m.kind.clone()), Scalar::Text(k.clone()), Scalar::Text(v.clone())]).collect() };
    o.summary.metrics = metrics;
    o.ports.push(PortData::Table(Rc::new(mt)));
    o.ports.push(PortData::Table(Rc::new(pt)));
    Ok(o)
}

fn class_metric_rows(cm: &ml::ClassMetrics) -> Vec<(String, String)> {
    let mut v = vec![("Accuracy".into(), format!("{:.1}%", cm.accuracy * 100.0)), ("Precision".into(), format!("{:.3}", cm.precision)), ("Recall".into(), format!("{:.3}", cm.recall)), ("F1".into(), format!("{:.3}", cm.f1))];
    if let Some(a) = cm.auc {
        v.push(("AUC".into(), format!("{a:.3}")));
    }
    v.push(("Rows".into(), fmt_int(cm.n)));
    v
}

fn confusion_table(labels: &[String], cm: &[Vec<usize>]) -> TableData {
    let mut columns = vec!["Actual \\ Predicted".to_string()];
    columns.extend(labels.iter().cloned());
    columns.push("Total".into());
    let rows = labels.iter().zip(cm).map(|(l, r)| std::iter::once(Scalar::Text(l.clone())).chain(r.iter().map(|v| Scalar::Num(*v as f64))).chain(std::iter::once(Scalar::Num(r.iter().sum::<usize>() as f64))).collect()).collect();
    let mut t = TableData { name: "Confusion matrix".into(), columns, types: vec![], rows };
    t.infer_types();
    t
}

// =============================================================================================
// CSV
// =============================================================================================

/// Picks the most likely delimiter from the first lines.
pub fn detect_delimiter(text: &str) -> char {
    let head: Vec<&str> = text.lines().take(10).collect();
    let mut best = (',', 0usize);
    for d in [',', '\t', ';', '|'] {
        let counts: Vec<usize> = head.iter().map(|l| l.matches(d).count()).collect();
        let min = counts.iter().copied().min().unwrap_or(0);
        if min > best.1 {
            best = (d, min);
        }
    }
    best.0
}

/// RFC 4180 CSV parsing (quoted fields, doubled quotes, CR/LF).
pub fn parse_csv(text: &str, delim: char) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            c if c == delim => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            c => field.push(c),
        }
    }
    if any && (!field.is_empty() || !row.is_empty()) {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// Writes a table as CSV (headers first).
pub fn write_csv(t: &TableData, delim: char) -> String {
    let esc = |s: &str| if s.contains(delim) || s.contains('"') || s.contains('\n') || s.contains('\r') { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() };
    let mut out = String::new();
    out.push_str(&t.columns.iter().map(|c| esc(c)).collect::<Vec<_>>().join(&delim.to_string()));
    out.push('\n');
    for r in &t.rows {
        let cells: Vec<String> = r
            .iter()
            .enumerate()
            .map(|(i, v)| match v {
                Scalar::Null => String::new(),
                Scalar::Num(n) if t.types.get(i).copied().unwrap_or_default().is_temporal() => crate::numfmt::format_date(*n, if n.fract() == 0.0 { "yyyy-mm-dd" } else { "yyyy-mm-dd hh:mm:ss" }),
                Scalar::Num(n) => format!("{n}"),
                Scalar::Bool(b) => if *b { "true".into() } else { "false".into() },
                Scalar::Text(s) => esc(s),
            })
            .collect();
        out.push_str(&cells.join(&delim.to_string()));
        out.push('\n');
    }
    out
}

/// A table from parsed CSV rows (types inferred; ISO dates become dates).
pub fn table_from_rows(name: &str, mut rows: Vec<Vec<String>>, header: bool) -> TableData {
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let columns: Vec<String> = if header && !rows.is_empty() {
        let h = rows.remove(0);
        (0..width).map(|i| h.get(i).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("Column{}", i + 1))).collect()
    } else {
        (0..width).map(|i| format!("Column{}", i + 1)).collect()
    };
    let mut data: Vec<Vec<Scalar>> = rows.iter().map(|r| (0..width).map(|i| crate::sheet::parse_input(r.get(i).map(String::as_str).unwrap_or(""))).collect()).collect();
    let mut types = vec![ColType::Any; width];
    for (c, ty) in types.iter_mut().enumerate() {
        let texts: Vec<&str> = data.iter().filter_map(|r| if let Some(Scalar::Text(t)) = r.get(c) { Some(t.as_str()) } else { None }).take(200).collect();
        let iso = |s: &str| s.len() >= 8 && s.chars().next().is_some_and(|ch| ch.is_ascii_digit()) && (s.contains('-') || s.contains('/')) && crate::build::parse_date(s).is_some();
        if !texts.is_empty() && texts.iter().all(|s| iso(s)) && data.iter().all(|r| !matches!(r.get(c), Some(Scalar::Num(_)) | Some(Scalar::Bool(_)))) {
            *ty = ColType::Date;
            for r in data.iter_mut() {
                if let Some(v) = r.get_mut(c) {
                    *v = crate::build::convert(v, ColType::Date);
                }
            }
        }
    }
    let mut t = TableData { name: name.to_string(), columns, types, rows: data };
    t.infer_types();
    t
}

fn file_stem(path: &str) -> String {
    std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "data".into())
}

// =============================================================================================
// Python
// =============================================================================================

/// A Python run: previous code (output suppressed), the code, input tables (written to CSV
/// and loaded as pandas DataFrames, or lists of dicts without pandas).
#[derive(Clone, Debug, Default)]
pub struct PyJob {
    pub prelude: String,
    pub code: String,
    pub inputs: Vec<(String, TableData)>,
    /// Read `out_df` back (script nodes).
    pub want_out_df: bool,
    pub timeout_s: u64,
}

#[derive(Clone, Debug, Default)]
pub struct PyResult {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
    pub out_df: Option<TableData>,
    /// The value of a trailing expression: a table (DataFrame) or its repr.
    pub result_table: Option<TableData>,
    pub result_text: Option<String>,
    pub pandas: bool,
}

impl PyResult {
    pub fn log(&self) -> String {
        let mut s = self.stdout.clone();
        if !self.stderr.trim().is_empty() {
            if !s.is_empty() && !s.ends_with('\n') {
                s.push('\n');
            }
            s.push_str(&self.stderr);
        }
        s
    }
}

/// A Python identifier for a table name (`Sales Target` → `Sales_Target`).
pub fn py_ident(name: &str) -> String {
    let mut s: String = name.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if s.chars().next().is_none_or(|c| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    s
}

const PY_RUNNER: &str = r#"
import sys, os, io, ast, json, contextlib
_d = os.path.dirname(os.path.abspath(__file__))
_meta = json.load(open(os.path.join(_d, '_meta.json'), encoding='utf-8'))
try:
    import pandas as pd
    _PANDAS = True
except Exception:
    _PANDAS = False
import csv as _csv
def _load(path):
    if _PANDAS:
        return pd.read_csv(path)
    with open(path, newline='', encoding='utf-8') as f:
        return list(_csv.DictReader(f))
tables = {}
_g = {'__name__': '__main__', 'tables': tables}
for _name, _var, _file in _meta['inputs']:
    _df = _load(os.path.join(_d, _file))
    tables[_name] = _df
    _g[_var] = _df
if _meta['want_out_df']:
    _g['out_df'] = _g.get('in_df')
def _write(v, path):
    if _PANDAS and isinstance(v, pd.DataFrame):
        if not isinstance(v.index, pd.RangeIndex):
            v = v.reset_index()
        v.to_csv(path, index=False)
        return True
    if _PANDAS and isinstance(v, pd.Series):
        v.to_frame().reset_index().to_csv(path, index=False)
        return True
    if isinstance(v, list) and v and isinstance(v[0], dict):
        keys = list(v[0].keys())
        with open(path, 'w', newline='', encoding='utf-8') as f:
            w = _csv.DictWriter(f, fieldnames=keys)
            w.writeheader()
            for r in v:
                w.writerow(r)
        return True
    return False
_pre = open(os.path.join(_d, '_prelude.py'), encoding='utf-8').read()
if _pre.strip():
    with contextlib.redirect_stdout(io.StringIO()):
        try:
            exec(compile(_pre, '<previous cells>', 'exec'), _g)
        except Exception:
            pass
_src = open(os.path.join(_d, '_cell.py'), encoding='utf-8').read()
_tree = ast.parse(_src, '<cell>')
_last = None
if _tree.body and isinstance(_tree.body[-1], ast.Expr):
    _last = ast.Expression(_tree.body.pop().value)
exec(compile(_tree, '<cell>', 'exec'), _g)
if _last is not None:
    _v = eval(compile(_last, '<cell>', 'eval'), _g)
    if _v is not None:
        if not _write(_v, os.path.join(_d, '_result.csv')):
            open(os.path.join(_d, '_result.txt'), 'w', encoding='utf-8').write(repr(_v))
if _meta['want_out_df'] and _g.get('out_df') is not None:
    if not _write(_g['out_df'], os.path.join(_d, '_out_df.csv')):
        print('out_df must be a DataFrame or a list of dicts', file=sys.stderr)
print('\n__pandas__=' + str(_PANDAS), file=sys.stderr)
"#;

static PY_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Runs Python (`python3`, or `python` on Windows) in a temporary folder.
pub fn run_python(job: &PyJob) -> Result<PyResult, String> {
    let n = PY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("vizcraft-py-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't create a temporary folder: {e}"))?;
    let w = |name: &str, s: &str| std::fs::write(dir.join(name), s).map_err(|e| format!("Couldn't write {name}: {e}"));
    let mut meta_inputs = Vec::new();
    for (i, (name, t)) in job.inputs.iter().enumerate() {
        let file = format!("in{i}.csv");
        w(&file, &write_csv(t, ','))?;
        meta_inputs.push(json!([name, py_ident(name), file]));
    }
    w("_meta.json", &json!({ "inputs": meta_inputs, "want_out_df": job.want_out_df }).to_string())?;
    w("_prelude.py", &job.prelude)?;
    w("_cell.py", &job.code)?;
    w("_runner.py", PY_RUNNER)?;
    let exe = if cfg!(windows) { "python" } else { "python3" };
    let mut child = std::process::Command::new(exe)
        .arg(dir.join("_runner.py"))
        .current_dir(&dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't start {exe}: {e}. Install Python 3 (with pandas for DataFrames)."))?;
    // Read pipes on threads so a chatty script can't block.
    let mut so = child.stdout.take();
    let mut se = child.stderr.take();
    let t_out = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = so.as_mut() {
            let _ = std::io::Read::read_to_string(o, &mut s);
        }
        s
    });
    let t_err = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = se.as_mut() {
            let _ = std::io::Read::read_to_string(o, &mut s);
        }
        s
    });
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(job.timeout_s.max(1));
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
            Err(_) => break None,
        }
    };
    let stdout = t_out.join().unwrap_or_default();
    let mut stderr = t_err.join().unwrap_or_default();
    let pandas = stderr.contains("__pandas__=True");
    if let Some(p) = stderr.find("\n__pandas__=") {
        stderr.truncate(p);
    }
    let stderr = stderr.trim_end().to_string();
    let read_table = |name: &str| std::fs::read_to_string(dir.join(name)).ok().map(|s| table_from_rows("out_df", parse_csv(&s, ','), true));
    let mut r = PyResult { ok: status.is_some_and(|s| s.success()), stdout, stderr, pandas, ..PyResult::default() };
    if status.is_none() {
        r.stderr = format!("The script ran longer than {} s and was stopped.\n{}", job.timeout_s, r.stderr);
    }
    r.out_df = read_table("_out_df.csv");
    r.result_table = read_table("_result.csv");
    r.result_text = std::fs::read_to_string(dir.join("_result.txt")).ok();
    let _ = std::fs::remove_dir_all(&dir);
    Ok(r)
}

// =============================================================================================
// Model tables from flows
// =============================================================================================

/// Parses `flow::node` (node id, or label).
pub fn parse_spec(spec: &str) -> (String, String) {
    match spec.rsplit_once("::") {
        Some((f, n)) => (f.to_string(), n.to_string()),
        None => (spec.to_string(), String::new()),
    }
}

/// The spec string for a flow node.
pub fn spec(flow: &str, node: u64) -> String {
    format!("{flow}::{node}")
}

pub fn find_node<'a>(flow: &'a Flow, key: &str) -> Option<&'a FlowNode> {
    if let Ok(id) = key.parse::<u64>()
        && let Some(n) = flow.nodes.iter().find(|n| n.id == id)
    {
        return Some(n);
    }
    flow.nodes.iter().find(|n| n.label.eq_ignore_ascii_case(key)).or_else(|| {
        if key.is_empty() {
            flow.nodes.iter().find(|n| n.kind == "out.table")
        } else {
            flow.nodes.iter().find(|n| n.kind == "out.table" && s_str(n, "name").eq_ignore_ascii_case(key))
        }
    })
}

/// The output table of a flow node ("flow::node id").
pub fn table_for(p: &Project, spec: &str, host: &dyn Host) -> crate::Result<TableData> {
    let depth = FLOW_DEPTH.with(|d| d.get());
    if depth > 8 {
        return crate::err(format!("Flow output `{spec}` refers to itself."));
    }
    let (fname, nkey) = parse_spec(spec);
    let Some(flow) = p.flows.iter().find(|f| f.name.eq_ignore_ascii_case(&fname)) else { return crate::err(format!("There's no flow named `{fname}`.")) };
    let Some(node) = find_node(flow, &nkey) else { return crate::err(format!("Flow `{fname}` has no node `{nkey}`.")) };
    FLOW_DEPTH.with(|d| d.set(depth + 1));
    let mut ex = Executor::default();
    let ctx = Ctx { project: p, host, tables: None };
    let r = ex.run(&ctx, flow, node.id);
    FLOW_DEPTH.with(|d| d.set(depth));
    match r {
        Ok(o) => match o.table(0) {
            Some(t) => Ok((**t).clone()),
            None => crate::err(format!("Flow node `{}` has no table output.", node.label)),
        },
        Err(e) => crate::err(format!("Flow `{fname}` → {}: {e}", node.label)),
    }
}

// =============================================================================================
// Examples
// =============================================================================================

fn mk(p: &mut Project, kind: &str, label: &str, x: f32, y: f32, cfg: J) -> FlowNode {
    let config = match cfg {
        J::Object(o) => o.into_iter().collect(),
        _ => BTreeMap::new(),
    };
    FlowNode { id: p.next_id(), kind: kind.into(), label: label.into(), x, y, config }
}

/// "Customer segments": Sales (sheet) ⋈ Product → line sales → totals per store and product →
/// margin → store names → k-means on [Total Sales, Margin] → saved as a model table.
pub fn example_segments(p: &mut Project) -> Flow {
    let name = unique_flow_name(p, "Customer segments");
    let x = |i: f32| 40.0 + i * 196.0;
    let sales = mk(p, "source.sheet", "Sales", x(0.0), 40.0, json!({"name": "Sales"}));
    let product = mk(p, "source.query", "Product", x(0.0), 170.0, json!({"name": "Product"}));
    let join = mk(p, "tf.join", "Join products", x(1.0), 104.0, json!({"left_keys": ["ProductID"], "right_keys": ["ProductID"], "kind": "inner"}));
    let line = mk(p, "tf.formula", "Line sales", x(2.0), 104.0, json!({"name": "Line Sales", "expression": "=[Quantity] * [Unit Price] * (1 - [Discount])"}));
    let group = mk(
        p,
        "tf.group",
        "Sales by store",
        x(3.0),
        104.0,
        json!({"by": ["StoreID", "Product"], "aggregates": [{"column": "Line Sales", "op": "sum", "name": "Total Sales"}, {"column": "Quantity", "op": "sum", "name": "Units"}, {"column": "Unit Cost", "op": "first", "name": "Unit Cost"}, {"column": "OrderID", "op": "count_distinct", "name": "Orders"}]}),
    );
    let margin = mk(p, "tf.formula", "Margin", x(4.0), 104.0, json!({"name": "Margin", "expression": "=([Total Sales] - [Units] * [Unit Cost]) / [Total Sales]"}));
    let store = mk(p, "source.query", "Store", x(4.0), 240.0, json!({"name": "Store"}));
    let stores = mk(p, "tf.join", "Store names", x(5.0), 160.0, json!({"left_keys": ["StoreID"], "right_keys": ["StoreID"], "kind": "left"}));
    let km = mk(p, "ml.kmeans", "k-Means", x(6.0), 104.0, json!({"columns": ["Total Sales", "Margin"], "k": 4, "normalize": true, "seed": 7, "output": "Segment"}));
    let stats = mk(p, "stat.stats", "Feature statistics", x(6.0), 240.0, json!({}));
    let save = mk(p, "out.table", "Save as model table", x(7.0), 104.0, json!({"name": "Store Product Segments"}));
    let links = vec![
        (sales.id, 0, join.id, 0),
        (product.id, 0, join.id, 1),
        (join.id, 0, line.id, 0),
        (line.id, 0, group.id, 0),
        (group.id, 0, margin.id, 0),
        (margin.id, 0, stores.id, 0),
        (store.id, 0, stores.id, 1),
        (stores.id, 0, km.id, 0),
        (stores.id, 0, stats.id, 0),
        (km.id, 0, save.id, 0),
    ];
    Flow { name, nodes: vec![sales, product, join, line, group, margin, store, stores, km, stats, save], links, description: "Segments store × product pairs by sales and margin with k-means.".into() }
}

/// "Predict order value": a random forest regression of line sales, scored on a test split.
pub fn example_regression(p: &mut Project) -> Flow {
    let name = unique_flow_name(p, "Predict line sales");
    let sales = mk(p, "source.sheet", "Sales", 40.0, 120.0, json!({"name": "Sales"}));
    let line = mk(p, "tf.formula", "Line sales", 236.0, 120.0, json!({"name": "Line Sales", "expression": "=[Quantity] * [Unit Price] * (1 - [Discount])"}));
    let split = mk(p, "ml.split", "Train/test split", 432.0, 120.0, json!({"train": 0.75, "seed": 1}));
    let forest = mk(p, "ml.forest", "Random forest", 628.0, 40.0, json!({"target": "Line Sales", "features": ["Quantity", "Unit Price", "Discount"], "trees": 30, "max_depth": 12, "task": "regression"}));
    let linreg = mk(p, "ml.linreg", "Linear regression", 628.0, 220.0, json!({"target": "Line Sales", "features": ["Quantity", "Unit Price", "Discount"]}));
    let score = mk(p, "ml.score", "Test & Score", 824.0, 40.0, json!({}));
    let score2 = mk(p, "ml.score", "Test & Score (linear)", 824.0, 220.0, json!({}));
    let links = vec![(sales.id, 0, line.id, 0), (line.id, 0, split.id, 0), (split.id, 0, forest.id, 0), (split.id, 0, linreg.id, 0), (forest.id, 0, score.id, 0), (split.id, 1, score.id, 1), (linreg.id, 0, score2.id, 0), (split.id, 1, score2.id, 1)];
    Flow { name, nodes: vec![sales, line, split, forest, linreg, score, score2], links, description: "Compares a random forest and a linear model on held-out orders.".into() }
}

pub fn unique_flow_name(p: &Project, base: &str) -> String {
    if !p.flows.iter().any(|f| f.name.eq_ignore_ascii_case(base)) {
        return base.to_string();
    }
    (2..1000).map(|i| format!("{base} {i}")).find(|n| !p.flows.iter().any(|f| f.name.eq_ignore_ascii_case(n))).unwrap_or_else(|| base.to_string())
}

// =============================================================================================
// Tests
// =============================================================================================

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use vizcraft_query::NoHost;

    fn t(cols: &[&str], rows: Vec<Vec<Scalar>>) -> TableData {
        let mut t = TableData { name: "t".into(), columns: cols.iter().map(|s| s.to_string()).collect(), types: vec![], rows };
        t.infer_types();
        t
    }
    fn n(x: f64) -> Scalar {
        Scalar::Num(x)
    }
    fn s(x: &str) -> Scalar {
        Scalar::Text(x.into())
    }

    #[test]
    fn catalog_is_consistent() {
        let c = catalog();
        let mut ids = HashSet::new();
        for nt in &c {
            assert!(ids.insert(nt.id), "duplicate {}", nt.id);
            assert!(CATEGORIES.contains(&nt.category), "{}", nt.id);
            assert!(!nt.outputs.is_empty(), "{}", nt.id);
        }
        assert!(c.len() >= 35);
    }

    #[test]
    fn csv_roundtrip() {
        let rows = parse_csv("a,b\n1,\"x, \"\"y\"\"\"\n2,z\r\n", ',');
        assert_eq!(rows, vec![vec!["a", "b"], vec!["1", "x, \"y\""], vec!["2", "z"]]);
        let tb = table_from_rows("t", rows, true);
        assert_eq!(tb.types[0], ColType::Integer);
        let back = write_csv(&tb, ',');
        assert_eq!(back, "a,b\n1,\"x, \"\"y\"\"\"\n2,z\n");
        assert_eq!(detect_delimiter("a;b;c\n1;2;3"), ';');
        let d = table_from_rows("d", parse_csv("when\n2024-01-31\n2024-02-01\n", ','), true);
        assert_eq!(d.types[0], ColType::Date);
        assert_eq!(d.rows[0][0], n(45322.0));
    }

    #[test]
    fn group_and_join() {
        let a = t(&["k", "v"], vec![vec![s("x"), n(1.0)], vec![s("y"), n(2.0)], vec![s("x"), n(3.0)]]);
        let g = group_by(&a, &["k".into()], &[("v".into(), "sum".into(), "Total".into()), (String::new(), "count".into(), String::new())]).unwrap();
        assert_eq!(g.columns, vec!["k", "Total", "Count rows"]);
        assert_eq!(g.rows, vec![vec![s("x"), n(4.0), n(2.0)], vec![s("y"), n(2.0), n(1.0)]]);
        let b = t(&["k", "w"], vec![vec![s("x"), s("ex")], vec![s("z"), s("zed")]]);
        let inner = join(&a, &b, &["k".into()], &["k".into()], "inner").unwrap();
        assert_eq!(inner.columns, vec!["k", "v", "w"]);
        assert_eq!(inner.rows.len(), 2);
        let full = join(&a, &b, &["k".into()], &["k".into()], "full").unwrap();
        assert_eq!(full.rows.len(), 4);
        assert_eq!(full.rows[3], vec![s("z"), Scalar::Null, s("zed")]);
        let left = join(&a, &b, &["k".into()], &["k".into()], "left").unwrap();
        assert_eq!(left.rows.len(), 3);
    }

    fn flow_of(nodes: Vec<FlowNode>, links: Vec<(u64, usize, u64, usize)>) -> Flow {
        Flow { name: "F".into(), nodes, links, description: String::new() }
    }
    fn node(id: u64, kind: &str, cfg: J) -> FlowNode {
        FlowNode { id, kind: kind.into(), label: kind.into(), x: 0.0, y: 0.0, config: match cfg {
            J::Object(o) => o.into_iter().collect(),
            _ => BTreeMap::new(),
        } }
    }

    #[test]
    fn executor_runs_and_caches() {
        let mut p = Project::default();
        p.sheets.push(crate::doc::DataSheet { name: "S".into(), cells: vec![vec!["g".into(), "x".into(), "y".into()], vec!["a".into(), "1".into(), "=B2*2".into()], vec!["b".into(), "5".into(), "=B3*2".into()], vec!["a".into(), "".into(), "7".into()]], ..Default::default() });
        let f = flow_of(
            vec![
                node(1, "source.sheet", json!({"name": "S"})),
                node(2, "tf.impute", json!({"columns": ["x"], "method": "constant", "value": "10"})),
                node(3, "tf.formula", json!({"name": "z", "expression": "=[x] + [y]"})),
                node(4, "tf.filter", json!({"conditions": [{"column": "z", "op": ">", "value": "5"}]})),
                node(5, "tf.sort", json!({"keys": [{"column": "z", "desc": true}]})),
            ],
            vec![(1, 0, 2, 0), (2, 0, 3, 0), (3, 0, 4, 0), (4, 0, 5, 0)],
        );
        let ctx = Ctx { project: &p, host: &NoHost, tables: None };
        let mut ex = Executor::default();
        let o = ex.run(&ctx, &f, 5).unwrap();
        let tb = o.table(0).unwrap();
        assert_eq!(tb.column_index("z"), Some(3));
        assert_eq!(tb.rows.iter().map(|r| r[3].clone()).collect::<Vec<_>>(), vec![n(17.0), n(15.0)]);
        let st = ex.statuses(&ctx, &f);
        assert_eq!(st[&5], Status::Ok);
        // Changing a setting makes it (and downstream) stale.
        let mut f2 = f.clone();
        f2.nodes[3].config.insert("conditions".into(), json!([{"column": "z", "op": ">", "value": "16"}]));
        let st = ex.statuses(&ctx, &f2);
        assert_eq!(st[&3], Status::Ok);
        assert_eq!(st[&4], Status::Stale);
        assert_eq!(st[&5], Status::Stale);
        assert_eq!(ex.run(&ctx, &f2, 5).unwrap().table(0).unwrap().rows.len(), 1);
        // A missing input is an error.
        let f3 = flow_of(vec![node(1, "tf.sort", json!({}))], vec![]);
        assert!(ex.run(&ctx, &f3, 1).unwrap_err().contains("Connect"));
    }

    #[test]
    fn sample_examples_run_on_the_retail_project() {
        let mut p = crate::sample::sales();
        let seg = example_segments(&mut p);
        let reg = example_regression(&mut p);
        p.flows.push(seg.clone());
        p.flows.push(reg.clone());
        let ctx = Ctx { project: &p, host: &NoHost, tables: None };
        let mut ex = Executor::default();
        assert_eq!(ex.run_all(&ctx, &seg), 0, "{:?}", seg.nodes.iter().map(|n| (n.label.clone(), ex.cached(&seg.name, n.id).map(|r| r.as_ref().err().cloned()))).collect::<Vec<_>>());
        let save = seg.nodes.iter().find(|n| n.kind == "out.table").unwrap();
        let out = ex.run(&ctx, &seg, save.id).unwrap();
        let tb = out.table(0).unwrap();
        assert_eq!(tb.rows.len(), 96);
        assert!(tb.column_index("Segment").is_some() && tb.column_index("Store").is_some());
        let km = seg.nodes.iter().find(|n| n.kind == "ml.kmeans").unwrap();
        let kmo = ex.run(&ctx, &seg, km.id).unwrap();
        assert!(kmo.summary.scatter.as_ref().unwrap().points.len() == 96);
        // As a model table.
        let data = table_for(&p, &spec(&seg.name, save.id), &NoHost).unwrap();
        assert_eq!(data.rows.len(), 96);
        assert_eq!(ex.run_all(&ctx, &reg), 0);
        let sc = reg.nodes.iter().find(|n| n.kind == "ml.score").unwrap();
        let so = ex.run(&ctx, &reg, sc.id).unwrap();
        let r2: f64 = so.summary.metrics.iter().find(|m| m.0 == "R²").unwrap().1.parse().unwrap();
        assert!(r2 > 0.85, "R² {r2}");
    }

    #[test]
    fn flow_outputs_load_into_the_model() {
        let mut p = crate::sample::sales();
        let f = p.flows.iter().find(|f| f.name == "Customer segments").cloned().unwrap();
        let save = f.nodes.iter().find(|n| n.kind == "out.table").unwrap();
        p.model.tables.push(crate::doc::ModelTable { name: "Segments".into(), source: TableSource::Flow(spec(&f.name, save.id)), ..Default::default() });
        crate::build::sync_model_tables(&mut p);
        let built = crate::build::build(&p, &NoHost, &BTreeMap::new());
        assert!(built.errors.iter().all(|e| e.0 != "Segments"), "{:?}", built.errors);
        assert_eq!(built.tables.get("Segments").unwrap().rows.len(), 96);
        assert!(built.model.table("Segments").is_some());
    }

    #[test]
    fn classification_pipeline() {
        let mut rng = ml::Rng::new(5);
        let rows: Vec<Vec<Scalar>> = (0..200)
            .map(|i| {
                let c = i % 3;
                vec![n(c as f64 * 3.0 + rng.f64()), n(rng.f64()), s(["red", "green", "blue"][c])]
            })
            .collect();
        let data = Rc::new(t(&["a", "b", "label"], rows));
        let mut p = Project::default();
        p.sheets.push(crate::doc::DataSheet { name: "D".into(), cells: std::iter::once(vec!["a".to_string(), "b".into(), "label".into()]).chain(data.rows.iter().map(|r| r.iter().map(|v| v.text()).collect())).collect(), ..Default::default() });
        for learner in ["ml.logreg", "ml.tree", "ml.forest", "ml.nb", "ml.knn"] {
            let f = flow_of(
                vec![
                    node(1, "source.sheet", json!({"name": "D"})),
                    node(2, "ml.split", json!({"train": 0.7, "stratify": "label"})),
                    node(3, learner, json!({"target": "label"})),
                    node(4, "ml.score", json!({})),
                    node(5, "ml.predict", json!({})),
                    node(6, "ml.confusion", json!({"actual": "label", "predicted": "Prediction"})),
                ],
                vec![(1, 0, 2, 0), (2, 0, 3, 0), (3, 0, 4, 0), (2, 1, 4, 1), (3, 0, 5, 0), (2, 1, 5, 1), (5, 0, 6, 0)],
            );
            let ctx = Ctx { project: &p, host: &NoHost, tables: None };
            let mut ex = Executor::default();
            let o = ex.run(&ctx, &f, 4).unwrap();
            let acc = &o.summary.metrics[0];
            assert_eq!(acc.0, "Accuracy");
            let v: f64 = acc.1.trim_end_matches('%').parse().unwrap();
            assert!(v > 90.0, "{learner}: {v}");
            let cm = ex.run(&ctx, &f, 6).unwrap();
            assert_eq!(cm.table(0).unwrap().rows.len(), 3);
        }
    }

    #[test]
    fn transforms() {
        let a = Rc::new(t(&["k", "m", "v"], vec![vec![s("x"), s("jan"), n(1.0)], vec![s("x"), s("feb"), n(2.0)], vec![s("y"), s("jan"), n(3.0)], vec![s("y"), s("jan"), n(3.0)]]));
        let run = |kind: &str, cfg: J| exec(&Ctx { project: &Project::default(), host: &NoHost, tables: None }, &Flow::default(), &node(1, kind, cfg), &[Some(PortData::Table(a.clone()))]).unwrap();
        let pv = run("tf.pivot", json!({"rows": ["k"], "column": "m", "value": "v", "op": "sum"}));
        let pt = pv.table(0).unwrap();
        assert_eq!(pt.columns, vec!["k", "feb", "jan"]);
        assert_eq!(pt.rows[1], vec![s("y"), Scalar::Null, n(6.0)]);
        let up = run("tf.unpivot", json!({"keep": ["k"]}));
        assert_eq!(up.table(0).unwrap().rows.len(), 8);
        assert_eq!(run("tf.dedupe", json!({})).table(0).unwrap().rows.len(), 3);
        let bin = run("tf.bin", json!({"column": "v", "bins": 2}));
        assert_eq!(bin.table(0).unwrap().rows[0][3], s("01. 1 – 2"));
        let norm = run("tf.normalize", json!({"method": "minmax"}));
        assert_eq!(norm.table(0).unwrap().rows[0][2], n(0.0));
        let smp = run("tf.sample", json!({"mode": "random", "n": 2, "seed": 3}));
        assert_eq!(smp.table(0).unwrap().rows.len() + smp.table(1).unwrap().rows.len(), 4);
        let sel = run("tf.select", json!({"columns": ["v"], "mode": "remove"}));
        assert_eq!(sel.table(0).unwrap().columns, vec!["k", "m"]);
        let st = run("stat.stats", json!({}));
        assert_eq!(st.table(0).unwrap().rows.len(), 3);
        let d = run("stat.dist", json!({"column": "k"}));
        assert_eq!(d.summary.bars.as_ref().unwrap().values, vec![2.0, 2.0]);
    }

    #[test]
    fn python_node_when_available() {
        if std::process::Command::new("python3").arg("--version").output().is_err() {
            return;
        }
        let a = Rc::new(t(&["x"], vec![vec![n(1.0)], vec![n(2.0)]]));
        let nd = node(1, "script.python", json!({"code": "print('hello')\nif hasattr(in_df, 'assign'):\n    out_df = in_df.assign(y=in_df['x'] * 10)\nelse:\n    out_df = [dict(r, y=float(r['x']) * 10) for r in in_df]\n"}));
        let o = exec(&Ctx { project: &Project::default(), host: &NoHost, tables: None }, &Flow::default(), &nd, &[Some(PortData::Table(a)), None]).unwrap();
        let tb = o.table(0).unwrap();
        assert_eq!(tb.columns, vec!["x", "y"]);
        assert_eq!(tb.rows[1][1], n(20.0));
        assert!(o.summary.log.contains("hello"));
        let r = run_python(&PyJob { prelude: "a = 41".into(), code: "b = a + 1\nb".into(), timeout_s: 30, ..PyJob::default() }).unwrap();
        assert_eq!(r.result_text.as_deref(), Some("42"));
        let bad = run_python(&PyJob { code: "raise ValueError('boom')".into(), timeout_s: 30, ..PyJob::default() }).unwrap();
        assert!(!bad.ok && bad.stderr.contains("boom"));
    }
}
