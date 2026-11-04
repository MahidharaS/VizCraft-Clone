//! Data Formulator-style derivations (after Microsoft's Data Formulator, MIT): a chart is a
//! *concept encoding* (x, y, color, size, facets → fields) plus a natural-language prompt. Fields
//! named in the encoding that don't exist yet ("Profit ratio") are *derived*: as DAX measures or
//! calculated columns, written by an LLM or by rules, and validated by running them.
//!
//! The app keeps the branching thread of steps; this module turns one step into a visual spec
//! over a scratch copy of the project and computes its data.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vizcraft_core::build::{self, Built};
use vizcraft_core::doc::*;
use vizcraft_core::state::{self, Runtime};
use vizcraft_core::vquery::{self, VisualData};

use crate::brain::features::{self, FieldSig};
use crate::tools::{field_use, parse_kind, resolve_field};

/// Encoding channels, as field names ("Table[Column]", "[Measure]" or a new name to derive).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Encoding {
    pub x: Vec<String>,
    pub y: Vec<String>,
    pub color: Vec<String>,
    pub size: Vec<String>,
    /// Facets (small multiples).
    pub column: Vec<String>,
    pub row: Vec<String>,
    /// Chart type (a `VisualKind` serde name), or "" for automatic.
    pub chart: String,
}

impl Encoding {
    pub const CHANNELS: [&'static str; 6] = ["x", "y", "color", "size", "column", "row"];
    pub fn channel(&self, c: &str) -> &Vec<String> {
        match c {
            "x" => &self.x,
            "y" => &self.y,
            "color" => &self.color,
            "size" => &self.size,
            "column" => &self.column,
            _ => &self.row,
        }
    }
    pub fn channel_mut(&mut self, c: &str) -> &mut Vec<String> {
        match c {
            "x" => &mut self.x,
            "y" => &mut self.y,
            "color" => &mut self.color,
            "size" => &mut self.size,
            "column" => &mut self.column,
            _ => &mut self.row,
        }
    }
    pub fn all(&self) -> impl Iterator<Item = &String> {
        self.x.iter().chain(&self.y).chain(&self.color).chain(&self.size).chain(&self.column).chain(&self.row)
    }
    pub fn is_empty(&self) -> bool {
        self.all().next().is_none()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DerivedField {
    pub name: String,
    pub dax: String,
    /// "measure" or "column".
    pub kind: String,
    pub table: String,
    /// Validation error, if it failed.
    pub error: Option<String>,
}

/// Fields in the encoding that aren't in the model (to derive).
pub fn unknown_fields(p: &Project, built: &Built, enc: &Encoding, derived: &[DerivedField]) -> Vec<String> {
    let mut v: Vec<String> = enc
        .all()
        .filter(|f| resolve_field(p, built, strip_agg(f)).is_none() && !derived.iter().any(|d| d.name.eq_ignore_ascii_case(strip_agg(f).trim_matches(['[', ']']))))
        .cloned()
        .collect();
    v.dedup();
    v
}

fn strip_agg(s: &str) -> &str {
    match s.split_once(':') {
        Some((a, f)) if !a.contains('[') => f,
        _ => s,
    }
}

/// A copy of the project with the derived fields added (measures / calculated columns).
pub fn with_derived(p: &Project, derived: &[DerivedField]) -> Project {
    let mut q = p.clone();
    let home = p.model.measures.first().map(|m| m.table.clone()).or_else(|| p.model.tables.first().map(|t| t.name.clone())).unwrap_or_default();
    for d in derived.iter().filter(|d| d.error.is_none()) {
        let table = if d.table.is_empty() { home.clone() } else { d.table.clone() };
        if d.kind == "column" {
            q.model.calculated_columns.retain(|c| !c.name.eq_ignore_ascii_case(&d.name));
            q.model.calculated_columns.push(CalcColumn { table, name: d.name.clone(), expression: d.dax.clone(), origin: "Formulator".into() });
        } else {
            q.model.measures.retain(|m| !m.name.eq_ignore_ascii_case(&d.name));
            q.model.measures.push(Measure { table, name: d.name.clone(), expression: d.dax.clone(), origin: "Formulator".into(), ..Measure::default() });
        }
    }
    q
}

/// Validates derived fields by building them; failing ones get `error` set.
pub fn validate(p: &Project, built: &Built, derived: &mut [DerivedField]) -> (Project, Built) {
    for d in derived.iter_mut() {
        d.error = None;
        let q = with_derived(p, std::slice::from_ref(d));
        let b = build::rebuild(&q, built, &crate::tools::params(&q));
        if let Some((_, e)) = b.errors.iter().find(|(o, _)| o.contains(&d.name)) {
            d.error = Some(e.clone());
            continue;
        }
        if d.kind != "column"
            && let Err(e) = b.model.evaluate(&format!("[{}]", d.name))
        {
            d.error = Some(e.to_string());
        }
    }
    let q = with_derived(p, derived);
    let b = build::rebuild(&q, built, &crate::tools::params(&q));
    (q, b)
}

fn sig(p: &Project, built: &Built, f: &str) -> Option<FieldSig> {
    let fu = field_use(p, built, "axis", strip_agg(f))?;
    Some(features::classify_use(p, built, &fu))
}

/// Field signatures of an encoding (for the brain's chart recommendation).
pub fn signatures(p: &Project, built: &Built, enc: &Encoding) -> Vec<FieldSig> {
    enc.all().filter_map(|f| sig(p, built, f)).collect()
}

/// Builds a visual spec from an encoding over `p` (which already holds derived fields).
/// `chart` overrides the encoding's chart type. Returns the spec and fields it couldn't place.
pub fn encoding_to_spec(p: &Project, built: &Built, enc: &Encoding, chart: VisualKind) -> (VisualSpec, Vec<String>) {
    let mut spec = VisualSpec::new(chart);
    let mut missing = Vec::new();
    let roles: Vec<&str> = chart.wells().iter().map(|(r, _)| *r).collect();
    let has = |r: &str| roles.contains(&r);
    let mut put = |spec: &mut VisualSpec, role: &str, f: &str, value: bool| {
        let r = if value { "values" } else { role };
        match field_use(p, built, r, f) {
            Some(mut fu) => {
                if value && !fu.is_value()
                    && let FieldRef::Column { table, column } = &fu.field
                {
                    fu.agg = state::default_agg(p, built, table, column);
                }
                spec.well_mut(role).push(fu);
            }
            None => missing.push(f.to_string()),
        }
    };
    let is_val = |f: &str| field_use(p, built, "values", f).is_some_and(|fu| fu.field.is_measure() || matches!(&fu.field, FieldRef::Column { table, column } if built.col_type(table, column).is_numeric() && !column.to_lowercase().contains("year"))) || f.contains(':') && !f.contains("::");
    // Categories and values from x / y (either orientation).
    let (mut cats, mut vals): (Vec<String>, Vec<String>) = (vec![], vec![]);
    for f in enc.x.iter().chain(enc.y.iter()) {
        if is_val(f) { vals.push(f.clone()) } else { cats.push(f.clone()) }
    }
    use VisualKind::*;
    match chart {
        Scatter => {
            for f in &enc.x {
                put(&mut spec, if is_val(f) { "x" } else { "details" }, f, is_val(f));
            }
            for f in &enc.y {
                put(&mut spec, "y", f, true);
            }
            for f in &enc.color {
                put(&mut spec, if is_val(f) { "size" } else { "details" }, f, is_val(f));
            }
            for f in &enc.size {
                put(&mut spec, "size", f, true);
            }
        }
        Pie | Donut | Treemap | Funnel | WordCloud | PackedBubbles => {
            let cat_role = if has("legend") && !has("axis") { "legend" } else { "axis" };
            for f in cats.iter().chain(enc.color.iter()) {
                put(&mut spec, cat_role, f, false);
            }
            for f in vals.iter().chain(enc.size.iter()) {
                put(&mut spec, "values", f, true);
            }
        }
        Card | MultiRowCard | Gauge | Kpi => {
            for f in &vals {
                put(&mut spec, "values", f, true);
            }
            if chart == Kpi {
                for f in &cats {
                    put(&mut spec, "axis", f, false);
                }
            }
        }
        Table => {
            for f in enc.all() {
                let v = is_val(f);
                put(&mut spec, "columns", f, v);
            }
        }
        Matrix | HeatMap | HighlightTable => {
            for f in &cats {
                put(&mut spec, "rows", f, false);
            }
            for f in enc.color.iter().chain(enc.column.iter()) {
                if is_val(f) { put(&mut spec, "values", f, true) } else { put(&mut spec, "columns", f, false) }
            }
            for f in &vals {
                put(&mut spec, "values", f, true);
            }
        }
        Map | FilledMap => {
            for f in &cats {
                put(&mut spec, "location", f, false);
            }
            for f in vals.iter().chain(enc.size.iter()) {
                put(&mut spec, "size", f, true);
            }
            for f in &enc.color {
                put(&mut spec, "legend", f, false);
            }
        }
        _ => {
            for f in &cats {
                put(&mut spec, "axis", f, false);
            }
            for f in &vals {
                put(&mut spec, "values", f, true);
            }
            // Colour by the axis field itself adds nothing (and would split every category into
            // one bar per colour); keep it a single series.
            for f in enc.color.iter().filter(|f| !cats.contains(f)) {
                if is_val(f) && has("secondary") { put(&mut spec, "secondary", f, true) } else { put(&mut spec, "legend", f, false) }
            }
            if has("small") {
                for f in enc.column.iter().chain(enc.row.iter()) {
                    put(&mut spec, "small", f, false);
                }
            }
            for f in &enc.size {
                put(&mut spec, "tooltips", f, true);
            }
        }
    }
    (spec, missing)
}

/// The data a step's chart is computed from (the "derived table" of the step).
pub fn compute(p: &Project, built: &Built, spec: &VisualSpec) -> VisualData {
    let page = p.report.pages.first().cloned().unwrap_or_default();
    let el = Element { kind: ElementKind::Visual(spec.clone()), ..Element::default() };
    let cx = state::context_for(p, built, &Runtime::default(), &page, &el);
    vquery::run(p, built, spec, &cx)
}

// ============================================================== rule-based derivation

fn numeric_fields(p: &Project, built: &Built) -> Vec<(String, String)> {
    // (display name, DAX of its total)
    let mut v: Vec<(String, String)> = p.model.measures.iter().map(|m| (m.name.clone(), format!("[{}]", m.name))).collect();
    for t in &p.model.tables {
        for (c, ty) in build::visible_columns(p, built, &t.name) {
            if ty.is_numeric() {
                v.push((c.clone(), format!("SUM({})", state::col_ref(&t.name, &c))));
            }
        }
    }
    v
}

/// Derives a field from its name with simple rules ("profit ratio", "sales per customer",
/// "average price", "revenue minus cost"). Returns None when the rules can't tell.
pub fn rules_derive(p: &Project, built: &Built, name: &str) -> Option<DerivedField> {
    let l = name.to_lowercase();
    let nums = numeric_fields(p, built);
    // Mentions, longest names first so "Total Sales" beats "Sales".
    let mut mentioned: Vec<(usize, &(String, String))> = Vec::new();
    let mut sorted: Vec<&(String, String)> = nums.iter().collect();
    sorted.sort_by_key(|(n, _)| std::cmp::Reverse(n.len()));
    let mut taken = l.clone();
    for f in sorted {
        let fl = f.0.to_lowercase();
        let words: Vec<&str> = fl.split_whitespace().collect();
        let key = words.last().copied().unwrap_or(fl.as_str());
        if let Some(pos) = taken.find(&fl).or_else(|| (key.len() > 2).then(|| taken.find(key)).flatten()) {
            mentioned.push((pos, f));
            let len = if taken[pos..].starts_with(&fl) { fl.len() } else { key.len() };
            taken.replace_range(pos..pos + len, &" ".repeat(len));
        }
    }
    mentioned.sort_by_key(|(p, _)| *p);
    let home = p.model.measures.first().map(|m| m.table.clone()).unwrap_or_default();
    let mk = |dax: String| Some(DerivedField { name: name.to_string(), dax, kind: "measure".into(), table: home.clone(), error: None });
    let a = mentioned.first().map(|(_, f)| f.1.clone());
    let b = mentioned.get(1).map(|(_, f)| f.1.clone());
    let is = |ws: &[&str]| ws.iter().any(|w| l.contains(w));
    match (a, b) {
        (Some(a), Some(b)) if is(&["ratio", " per ", " / ", "rate", "share", "%", "percent"]) => mk(format!("DIVIDE({a}, {b})")),
        (Some(a), Some(b)) if is(&["minus", "difference", " - ", "gap", "net"]) => mk(format!("{a} - {b}")),
        (Some(a), Some(b)) if is(&["plus", " + ", "combined", "sum"]) => mk(format!("{a} + {b}")),
        (Some(a), None) if is(&["margin", "ratio"]) => {
            // "<x> margin" → x / the largest revenue-like measure.
            let base = nums.iter().find(|(n, _)| ["sales", "revenue", "amount"].iter().any(|k| n.to_lowercase().contains(k)) && !a.contains(n.as_str()))?.1.clone();
            mk(format!("DIVIDE({a}, {base})"))
        }
        (Some(a), None) if is(&["average", "avg", "mean"]) => mk(a.replacen("SUM(", "AVERAGE(", 1)),
        (Some(a), None) if is(&["max", "highest", "peak"]) => mk(a.replacen("SUM(", "MAX(", 1)),
        (Some(a), None) if is(&["min", "lowest"]) => mk(a.replacen("SUM(", "MIN(", 1)),
        (Some(a), None) if is(&["running", "cumulative"]) => {
            let date = p.model.tables.iter().find_map(|t| build::visible_columns(p, built, &t.name).into_iter().find(|(_, ty)| ty.is_temporal()).map(|(c, _)| state::col_ref(&t.name, &c)))?;
            mk(format!("CALCULATE({a}, FILTER(ALLSELECTED({date}), {date} <= MAX({date})))"))
        }
        (Some(a), None) if is(&["share", "% of total", "percent of total", "contribution"]) => mk(format!("DIVIDE({a}, CALCULATE({a}, ALLSELECTED()))")),
        (None, None) if is(&["count", "number of", "rows"]) => {
            let t = p.model.tables.iter().find(|t| l.contains(&t.name.to_lowercase())).or_else(|| p.model.tables.iter().max_by_key(|t| built.table(&t.name).map(|d| d.len()).unwrap_or(0)))?;
            mk(format!("COUNTROWS({})", vizcraft_datamodel::model::quote_table(&t.name)))
        }
        _ => None,
    }
}

// ============================================================== LLM derivation

/// The prompt asking an LLM for an encoding + derived fields (JSON).
pub fn ai_prompt(p: &Project, built: &Built, prompt: &str, enc: &Encoding, parent: Option<&str>, habits: &str) -> String {
    let model = crate::tools::describe_model_json(p, built);
    let mut s = String::from(
        "You turn a data question into a chart, like Microsoft Data Formulator. The data is a DAX semantic model.\n\
         Answer with ONLY a JSON object:\n\
         {\"chart\": \"<line|clusteredColumn|clusteredBar|stackedColumn|area|scatter|pie|donut|treemap|card|table|matrix|heatMap|map>\",\n\
          \"encoding\": {\"x\": [fields], \"y\": [fields], \"color\": [fields], \"size\": [fields], \"column\": [fields], \"row\": [fields]},\n\
          \"derived\": [{\"name\": \"<new field>\", \"kind\": \"measure|column\", \"table\": \"<home table>\", \"dax\": \"<DAX expression>\"}],\n\
          \"explanation\": \"<one sentence>\"}\n\
         Fields are 'Table[Column]' or '[Measure]' or the name of a derived field. Derive any field the user names that doesn't exist \
         (aggregating measures for ratios/totals; calculated columns only for row-level values). Use only existing tables/columns in DAX.\n\n",
    );
    s.push_str(&format!("MODEL: {model}\n\n"));
    if !habits.is_empty() {
        s.push_str(&format!("{habits}\n\n"));
    }
    if let Some(par) = parent {
        s.push_str(&format!("PREVIOUS STEP (the user is refining it): {par}\n\n"));
    }
    if !enc.is_empty() {
        s.push_str(&format!("ENCODING THE USER SET (keep it, fill gaps): {}\n\n", serde_json::to_string(enc).unwrap_or_default()));
    }
    s.push_str(&format!("REQUEST: {prompt}\n"));
    s
}

/// Parses the LLM's answer (tolerant of missing parts).
pub fn parse_ai(v: &Value) -> (Encoding, Vec<DerivedField>, String) {
    let list = |k: &str| -> Vec<String> {
        match v.pointer(&format!("/encoding/{k}")) {
            Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
            Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
            _ => vec![],
        }
    };
    let enc = Encoding {
        x: list("x"),
        y: list("y"),
        color: list("color"),
        size: list("size"),
        column: list("column"),
        row: list("row"),
        chart: v.get("chart").and_then(Value::as_str).and_then(parse_kind).and_then(|k| serde_json::to_value(k).ok()).and_then(|x| x.as_str().map(str::to_string)).unwrap_or_default(),
    };
    let derived = v
        .get("derived")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|d| {
                    let name = d.get("name")?.as_str()?.trim().trim_matches(['[', ']']).to_string();
                    let dax = d.get("dax")?.as_str()?.trim().to_string();
                    (!name.is_empty() && !dax.is_empty()).then(|| DerivedField { name, dax, kind: d.get("kind").and_then(Value::as_str).unwrap_or("measure").to_string(), table: d.get("table").and_then(Value::as_str).unwrap_or("").to_string(), error: None })
                })
                .collect()
        })
        .unwrap_or_default();
    (enc, derived, v.get("explanation").and_then(Value::as_str).unwrap_or("").to_string())
}

/// Rule-based interpretation of a prompt into an encoding (no LLM): Q&A over the model.
pub fn rules_encoding(p: &Project, built: &Built, prompt: &str) -> Encoding {
    let (wells, _) = vizcraft_core::charts::qna::interpret(p, built, prompt);
    let name = |fu: &FieldUse| match &fu.field {
        FieldRef::Measure { name, .. } => format!("[{name}]"),
        FieldRef::Column { table, column } => {
            if fu.agg != Agg::None && fu.agg != Agg::Sum { format!("{}:{table}[{column}]", fu.agg.label()) } else { format!("{table}[{column}]") }
        }
        f => f.label(),
    };
    let mut enc = Encoding::default();
    let axis: Vec<String> = wells.get("axis").map(|v| v.iter().map(name).collect()).unwrap_or_default();
    enc.x = axis.first().cloned().into_iter().collect();
    // A second axis field becomes colour, unless it is the same column name from another table
    // (Store[Region] and Targets[Region] are one concept, not two).
    let col_name = |f: &str| f.rsplit('[').next().unwrap_or(f).trim_end_matches(']').to_lowercase();
    enc.color = axis.iter().skip(1).filter(|f| axis.first().is_none_or(|x| col_name(x) != col_name(f))).take(1).cloned().collect();
    enc.y = wells.get("values").map(|v| v.iter().map(name).collect()).unwrap_or_default();
    let l = prompt.to_lowercase();
    enc.chart = if l.contains("trend") || l.contains("over time") {
        "line".into()
    } else if l.contains("share") || l.contains("breakdown") || l.contains("pie") {
        "donut".into()
    } else if l.contains("table") || l.contains("list") {
        "table".into()
    } else {
        String::new()
    };
    let _ = json!(null);
    enc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (Project, Built) {
        let mut p = vizcraft_core::sample::sales();
        build::sync_model_tables(&mut p);
        let b = build::build(&p, &vizcraft_connectors::DesktopHost::default(), &crate::tools::params(&p));
        (p, b)
    }

    #[test]
    fn derive_validate_and_compute() {
        let (p, b) = sample();
        let enc = Encoding { x: vec!["Store[Region]".into()], y: vec!["Profit ratio".into()], ..Default::default() };
        let unknown = unknown_fields(&p, &b, &enc, &[]);
        assert_eq!(unknown, vec!["Profit ratio".to_string()]);
        let d = rules_derive(&p, &b, "Profit ratio").expect("rule");
        assert!(d.dax.starts_with("DIVIDE("), "{}", d.dax);
        let mut derived = vec![d, DerivedField { name: "Bad".into(), dax: "NOPE(".into(), kind: "measure".into(), table: String::new(), error: None }];
        let (q, qb) = validate(&p, &b, &mut derived);
        assert!(derived[0].error.is_none(), "{:?}", derived[0].error);
        assert!(derived[1].error.is_some());
        let enc2 = Encoding { y: vec!["[Profit ratio]".into()], ..enc };
        let (spec, missing) = encoding_to_spec(&q, &qb, &enc2, VisualKind::ClusteredColumn);
        assert!(missing.is_empty(), "{missing:?}");
        let data = compute(&q, &qb, &spec);
        assert!(data.error.is_none(), "{:?}", data.error);
        assert!(!data.rows.is_empty());
        // Signatures feed the brain.
        let sigs = signatures(&q, &qb, &enc2);
        assert_eq!(sigs.len(), 2);
    }

    #[test]
    fn color_same_as_axis_is_one_series() {
        let (p, b) = sample();
        let enc = Encoding { x: vec!["Store[Region]".into()], y: vec!["[Total Sales]".into()], color: vec!["Store[Region]".into()], ..Default::default() };
        let (spec, _) = encoding_to_spec(&p, &b, &enc, VisualKind::ClusteredColumn);
        assert!(spec.wells.get("legend").is_none_or(|v| v.is_empty()), "{:?}", spec.wells);
    }

    #[test]
    fn rule_encoding_and_ai_parsing() {
        let (p, b) = sample();
        let e = rules_encoding(&p, &b, "total sales by region");
        assert!(e.color.is_empty(), "same-named column from another table must not become colour: {e:?}");
        assert!(!e.y.is_empty() && !e.x.is_empty(), "{e:?}");
        let v = json!({"chart": "Line chart", "encoding": {"x": ["Sales[Date]"], "y": ["[Margin %]"]}, "derived": [{"name": "Margin %", "kind": "measure", "dax": "DIVIDE([Total Profit], [Total Sales])"}], "explanation": "Margin over time"});
        let (enc, der, why) = parse_ai(&v);
        assert_eq!(enc.chart, "line");
        assert_eq!(der[0].name, "Margin %");
        assert_eq!(why, "Margin over time");
        let (_, d2, _) = parse_ai(&json!({"garbage": 1}));
        assert!(d2.is_empty());
        for (enc, kind) in [(Encoding { x: vec!["Store[Region]".into()], y: vec!["[Total Sales]".into()], ..Default::default() }, VisualKind::Pie), (Encoding { x: vec!["[Total Sales]".into()], y: vec!["[Total Profit]".into()], ..Default::default() }, VisualKind::Scatter)] {
            let (spec, _) = encoding_to_spec(&p, &b, &enc, kind);
            assert!(compute(&p, &b, &spec).error.is_none(), "{kind:?}");
        }
    }
}
