//! Refresh: evaluates the Power Query queries, data sheets, flows and parameters into table
//! data, and builds the DAX model (tables, auto date/time columns, calculated tables,
//! relationships, calculated columns, measures).

use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;

use vizcraft_datamodel::store::Table as DmTable;
use vizcraft_datamodel::{CrossFilter, Model, Value as DmValue};
use vizcraft_query::m::eval::Host;
use vizcraft_query::{MType, Value as MValue};

use crate::doc::*;
use crate::scalar::Scalar;
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ColType {
    #[default]
    Any,
    Text,
    Number,
    Integer,
    Date,
    DateTime,
    Time,
    Bool,
}

impl ColType {
    pub fn label(self) -> &'static str {
        match self {
            ColType::Any => "Any",
            ColType::Text => "Text",
            ColType::Number => "Decimal Number",
            ColType::Integer => "Whole Number",
            ColType::Date => "Date",
            ColType::DateTime => "Date/Time",
            ColType::Time => "Time",
            ColType::Bool => "True/False",
        }
    }
    pub fn from_label(s: &str) -> ColType {
        match s {
            "Text" => ColType::Text,
            "Decimal Number" | "Fixed Decimal" | "Number" => ColType::Number,
            "Whole Number" => ColType::Integer,
            "Date" => ColType::Date,
            "Date/Time" => ColType::DateTime,
            "Time" => ColType::Time,
            "True/False" | "Boolean" => ColType::Bool,
            _ => ColType::Any,
        }
    }
    pub fn is_numeric(self) -> bool {
        matches!(self, ColType::Number | ColType::Integer)
    }
    pub fn is_temporal(self) -> bool {
        matches!(self, ColType::Date | ColType::DateTime)
    }
    pub fn default_format(self) -> &'static str {
        match self {
            ColType::Date => "yyyy-mm-dd",
            ColType::DateTime => "yyyy-mm-dd hh:mm",
            ColType::Time => "hh:mm:ss",
            ColType::Integer => "#,0",
            _ => "",
        }
    }
}

/// A table's rows, as loaded (before the DAX model encodes them).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct TableData {
    pub name: String,
    pub columns: Vec<String>,
    pub types: Vec<ColType>,
    pub rows: Vec<Vec<Scalar>>,
}

impl TableData {
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.eq_ignore_ascii_case(name))
    }
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub fn cell(&self, r: usize, c: usize) -> &Scalar {
        static NULL: Scalar = Scalar::Null;
        self.rows.get(r).and_then(|row| row.get(c)).unwrap_or(&NULL)
    }
    /// Infers column types from values (when a column has no declared type).
    pub fn infer_types(&mut self) {
        let n = self.columns.len();
        self.types.resize(n, ColType::Any);
        for c in 0..n {
            if self.types.get(c).copied().unwrap_or_default() != ColType::Any {
                continue;
            }
            let mut num = 0;
            let mut int = 0;
            let mut text = 0;
            let mut b = 0;
            for r in self.rows.iter().take(5000) {
                match r.get(c) {
                    Some(Scalar::Num(x)) => {
                        num += 1;
                        if x.fract() == 0.0 {
                            int += 1;
                        }
                    }
                    Some(Scalar::Text(_)) => text += 1,
                    Some(Scalar::Bool(_)) => b += 1,
                    _ => {}
                }
            }
            let t = if text > 0 {
                ColType::Text
            } else if b > 0 && num == 0 {
                ColType::Bool
            } else if num > 0 && int == num {
                ColType::Integer
            } else if num > 0 {
                ColType::Number
            } else {
                ColType::Text
            };
            if let Some(slot) = self.types.get_mut(c) {
                *slot = t;
            }
        }
    }
    /// Converts values to their columns' declared types (text dates in a Date column, …).
    pub fn normalize(&mut self) {
        for (c, ty) in self.types.clone().iter().enumerate() {
            if matches!(ty, ColType::Any) {
                continue;
            }
            for r in self.rows.iter_mut() {
                if let Some(v) = r.get_mut(c) {
                    let ok = match (ty, &*v) {
                        (_, Scalar::Null) => true,
                        (ColType::Text, Scalar::Text(_)) => true,
                        (ColType::Number | ColType::Integer | ColType::Date | ColType::DateTime | ColType::Time, Scalar::Num(_)) => true,
                        (ColType::Bool, Scalar::Bool(_)) => true,
                        _ => false,
                    };
                    if !ok {
                        *v = convert(v, *ty);
                    }
                }
            }
        }
    }

    /// From a Power Query table.
    pub fn from_m(name: &str, t: &vizcraft_query::Table) -> TableData {
        let types: Vec<ColType> = (0..t.columns.len())
            .map(|i| match t.types.get(i).copied().unwrap_or(MType::Any) {
                MType::Text => ColType::Text,
                MType::Number | MType::Currency | MType::Percentage => ColType::Number,
                MType::Int64 => ColType::Integer,
                MType::Date => ColType::Date,
                MType::DateTime => ColType::DateTime,
                MType::Time => ColType::Time,
                MType::Logical => ColType::Bool,
                _ => ColType::Any,
            })
            .collect();
        let mut out = TableData {
            name: name.to_string(),
            columns: t.columns.clone(),
            types,
            rows: t.rows.iter().map(|r| r.iter().map(m_to_scalar).collect()).collect(),
        };
        // Columns typed Any whose values are all dates are dates.
        for (c, ty) in out.types.iter_mut().enumerate() {
            if *ty == ColType::Any {
                let mut dates = 0;
                let mut other = 0;
                for r in t.rows.iter().take(2000) {
                    match r.get(c) {
                        Some(MValue::Date(_)) => dates += 1,
                        Some(MValue::DateTime(_)) => dates += 1,
                        Some(MValue::Null) | None => {}
                        _ => other += 1,
                    }
                }
                if dates > 0 && other == 0 {
                    *ty = ColType::Date;
                }
            }
        }
        out.infer_types();
        out
    }
}

pub fn m_to_scalar(v: &MValue) -> Scalar {
    match v {
        MValue::Null => Scalar::Null,
        MValue::Logical(b) => Scalar::Bool(*b),
        MValue::Number(n) => Scalar::Num(*n),
        MValue::Text(t) => Scalar::Text(t.to_string()),
        MValue::Date(d) => Scalar::Num(*d as f64),
        MValue::DateTime(x) | MValue::Time(x) | MValue::Duration(x) => Scalar::Num(*x),
        MValue::Error(e) => Scalar::Text(format!("Error: {e}")),
        other => Scalar::Text(other.display()),
    }
}

pub fn scalar_to_m(v: &Scalar) -> MValue {
    match v {
        Scalar::Null => MValue::Null,
        Scalar::Bool(b) => MValue::Logical(*b),
        Scalar::Num(n) => MValue::Number(*n),
        Scalar::Text(t) => MValue::text(t),
    }
}

impl TableData {
    /// As a Power Query table (sheets referenced from queries).
    pub fn to_m(&self) -> vizcraft_query::Table {
        let mut t = vizcraft_query::Table::new(self.columns.clone(), self.rows.iter().map(|r| r.iter().map(scalar_to_m).collect()).collect());
        t.types = self
            .types
            .iter()
            .map(|ty| match ty {
                ColType::Text => MType::Text,
                ColType::Number => MType::Number,
                ColType::Integer => MType::Int64,
                ColType::Date => MType::Date,
                ColType::DateTime => MType::DateTime,
                ColType::Time => MType::Time,
                ColType::Bool => MType::Logical,
                ColType::Any => MType::Any,
            })
            .collect();
        t
    }
}

/// Resolves other queries, parameters and data sheets by name for M, and passes files, web and
/// databases on to the outer host (the connectors).
pub struct ProjectHost<'a> {
    pub project: &'a Project,
    pub inner: &'a dyn Host,
    depth: Cell<usize>,
}

impl<'a> ProjectHost<'a> {
    pub fn new(project: &'a Project, inner: &'a dyn Host) -> ProjectHost<'a> {
        ProjectHost { project, inner, depth: Cell::new(0) }
    }
}

impl Host for ProjectHost<'_> {
    fn resolve(&self, name: &str) -> Option<vizcraft_query::Result<MValue>> {
        if self.depth.get() > 40 {
            return Some(Err(vizcraft_query::Error::Eval(format!("Queries refer to each other in a cycle ({name})."))));
        }
        if let Some(q) = self.project.queries.query(name) {
            self.depth.set(self.depth.get() + 1);
            let r = vizcraft_query::evaluate(&q.m, self);
            self.depth.set(self.depth.get().saturating_sub(1));
            return Some(r);
        }
        if let Some(p) = self.project.queries.parameters.iter().find(|p| p.name == name) {
            return Some(vizcraft_query::evaluate(&p.value, &vizcraft_query::NoHost));
        }
        if let Some(s) = self.project.sheet(name) {
            return Some(Ok(MValue::table(crate::sheet::evaluate_with(s, &self.project.sheets).to_m())));
        }
        self.inner.resolve(name)
    }
    fn current_workbook(&self) -> vizcraft_query::Result<vizcraft_query::Table> {
        // The project's data sheets play the part of the workbook's tables.
        let rows = self
            .project
            .sheets
            .iter()
            .map(|s| vec![MValue::table(crate::sheet::evaluate_with(s, &self.project.sheets).to_m()), MValue::text(&s.name)])
            .collect();
        Ok(vizcraft_query::Table::new(vec!["Content".into(), "Name".into()], rows))
    }
    fn file_contents(&self, path: &str) -> vizcraft_query::Result<Vec<u8>> {
        self.inner.file_contents(path)
    }
    fn excel_workbook(&self, bytes: &[u8], use_headers: bool) -> vizcraft_query::Result<vizcraft_query::Table> {
        self.inner.excel_workbook(bytes, use_headers)
    }
    fn folder_files(&self, path: &str) -> vizcraft_query::Result<vizcraft_query::Table> {
        self.inner.folder_files(path)
    }
    fn web_contents(&self, url: &str) -> vizcraft_query::Result<Vec<u8>> {
        self.inner.web_contents(url)
    }
    fn database(&self, kind: &str, args: &[String], query: Option<&str>) -> vizcraft_query::Result<vizcraft_query::Table> {
        self.inner.database(kind, args, query)
    }
}

/// Evaluates one query to a table.
pub fn evaluate_query(project: &Project, name: &str, host: &dyn Host) -> Result<vizcraft_query::Table> {
    let q = project.queries.query(name).ok_or_else(|| Error::Msg(format!("There's no query named `{name}`.")))?;
    let ph = ProjectHost::new(project, host);
    match vizcraft_query::evaluate(&q.m, &ph)? {
        MValue::Table(t) => Ok((*t).clone()),
        MValue::List(l) => Ok(vizcraft_query::Table::new(vec!["Value".into()], l.iter().map(|v| vec![v.clone()]).collect())),
        MValue::Record(r) => Ok(vizcraft_query::Table::new(r.names.clone(), vec![r.values.clone()])),
        other => Ok(vizcraft_query::Table::new(vec![name.to_string()], vec![vec![other]])),
    }
}

/// The result of a refresh.
#[derive(Clone, Debug, Default)]
pub struct Built {
    pub model: Rc<Model>,
    /// Loaded table data by table name (before auto date columns).
    pub tables: BTreeMap<String, Rc<TableData>>,
    /// Problems found (table or object → message). The model is still usable.
    pub errors: Vec<(String, String)>,
}

impl Built {
    pub fn col_type(&self, table: &str, column: &str) -> ColType {
        self.tables
            .get(table)
            .and_then(|t| t.column_index(column).and_then(|i| t.types.get(i).copied()))
            .unwrap_or(ColType::Any)
    }
    pub fn table(&self, name: &str) -> Option<&Rc<TableData>> {
        self.tables.get(name).or_else(|| self.tables.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v))
    }
}

/// The names of the auto date/time columns built for a date column.
pub const DATE_LEVELS: [&str; 4] = ["Year", "Quarter", "Month", "Day"];

pub fn date_level_column(column: &str, level: &str) -> String {
    format!("{column}.{level}")
}

/// Makes sure every table that should be in the model has a [`ModelTable`] entry: queries that
/// load, sheets that load, parameters. Keeps existing entries (and their metadata).
pub fn sync_model_tables(p: &mut Project) {
    let mut wanted: Vec<(String, TableSource)> = Vec::new();
    for q in &p.queries.queries {
        if q.load != vizcraft_query::steps::LoadTo::None {
            wanted.push((q.name.clone(), TableSource::Query(q.name.clone())));
        }
    }
    for s in &p.sheets {
        if s.load {
            wanted.push((s.name.clone(), TableSource::Sheet(s.name.clone())));
        }
    }
    for prm in &p.model.parameters {
        if matches!(prm.kind, ParameterKind::WhatIf { .. } | ParameterKind::Fields { .. }) {
            wanted.push((prm.name.clone(), TableSource::Parameter(prm.name.clone())));
        }
    }
    for (name, src) in &wanted {
        if !p.model.tables.iter().any(|t| t.name.eq_ignore_ascii_case(name)) {
            p.model.tables.push(ModelTable { name: name.clone(), source: src.clone(), storage_mode: "Import".into(), ..ModelTable::default() });
        }
    }
    // Drop tables whose source no longer exists (calculated / flow tables are kept).
    p.model.tables.retain(|t| match &t.source {
        TableSource::Query(_) | TableSource::Sheet(_) | TableSource::Parameter(_) => wanted.iter().any(|(_, s)| s == &t.source),
        _ => true,
    });
}

/// Loads every table's data. `host` reaches files, the web and databases.
pub fn load_tables(p: &Project, host: &dyn Host) -> (BTreeMap<String, Rc<TableData>>, Vec<(String, String)>) {
    let mut out = BTreeMap::new();
    let mut errors = Vec::new();
    for t in &p.model.tables {
        let data = match &t.source {
            TableSource::Query(q) => evaluate_query(p, q, host).map(|mt| TableData::from_m(&t.name, &mt)),
            TableSource::Sheet(s) => p.sheet(s).map(|sh| crate::sheet::evaluate_with(sh, &p.sheets)).ok_or_else(|| Error::Msg(format!("The data sheet `{s}` is missing."))),
            TableSource::Parameter(n) => parameter_table(p, n),
            TableSource::Flow(spec) => crate::flow::table_for(p, spec, host),
            TableSource::Calculated(_) | TableSource::None => continue,
        };
        match data {
            Ok(mut d) => {
                d.name = t.name.clone();
                d.normalize();
                apply_declared_types(&mut d, t);
                out.insert(t.name.clone(), Rc::new(d));
            }
            Err(e) => errors.push((t.name.clone(), e.to_string())),
        }
    }
    (out, errors)
}

fn apply_declared_types(d: &mut TableData, t: &ModelTable) {
    for cp in &t.columns {
        if cp.data_type.is_empty() {
            continue;
        }
        let Some(i) = d.column_index(&cp.name) else { continue };
        let ty = ColType::from_label(&cp.data_type);
        if ty == ColType::Any {
            continue;
        }
        if let Some(slot) = d.types.get_mut(i) {
            *slot = ty;
        }
        for r in d.rows.iter_mut() {
            if let Some(v) = r.get_mut(i) {
                *v = convert(v, ty);
            }
        }
    }
}

/// Converts a value to a column type (Table view "Data type").
pub fn convert(v: &Scalar, ty: ColType) -> Scalar {
    match (ty, v) {
        (_, Scalar::Null) => Scalar::Null,
        (ColType::Text, s) => Scalar::Text(match s {
            Scalar::Num(n) => crate::numfmt::general(*n),
            other => other.text(),
        }),
        (ColType::Number | ColType::Integer, s) => match s.as_f64().or_else(|| parse_date(&s.text())) {
            Some(n) => Scalar::Num(if ty == ColType::Integer { n.round() } else { n }),
            None => Scalar::Null,
        },
        (ColType::Date | ColType::DateTime, Scalar::Text(t)) => parse_date(t).map(Scalar::Num).unwrap_or(Scalar::Null),
        (ColType::Date, Scalar::Num(n)) => Scalar::Num(n.floor()),
        (ColType::Bool, s) => match s {
            Scalar::Bool(b) => Scalar::Bool(*b),
            Scalar::Num(n) => Scalar::Bool(*n != 0.0),
            Scalar::Text(t) => Scalar::Bool(matches!(t.trim().to_ascii_lowercase().as_str(), "true" | "yes" | "1" | "y")),
            Scalar::Null => Scalar::Null,
        },
        (_, s) => s.clone(),
    }
}

/// Parses `2024-01-31`, `2024/01/31`, `31/01/2024`, `01/31/2024`, `Jan 31 2024`, with optional time,
/// to a serial date.
pub fn parse_date(s: &str) -> Option<f64> {
    use chrono::{NaiveDate, NaiveDateTime};
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let base = NaiveDate::from_ymd_opt(1899, 12, 30)?;
    for f in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M", "%Y/%m/%d %H:%M:%S", "%m/%d/%Y %H:%M:%S", "%d/%m/%Y %H:%M"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, f) {
            let d = (dt.date() - base).num_days() as f64;
            let secs = dt.time().signed_duration_since(chrono::NaiveTime::MIN).num_seconds() as f64;
            return Some(d + secs / 86400.0);
        }
    }
    let s10 = s.get(..10).unwrap_or(s);
    for f in ["%Y-%m-%d", "%Y/%m/%d", "%m/%d/%Y", "%d/%m/%Y", "%d-%m-%Y", "%d.%m.%Y", "%b %d %Y", "%d %b %Y", "%B %d, %Y"] {
        if let Ok(d) = NaiveDate::parse_from_str(s10, f).or_else(|_| NaiveDate::parse_from_str(s, f)) {
            return Some((d - base).num_days() as f64);
        }
    }
    None
}

fn parameter_table(p: &Project, name: &str) -> Result<TableData> {
    let prm = p.parameter(name).ok_or_else(|| Error::Msg(format!("No parameter `{name}`.")))?;
    match &prm.kind {
        ParameterKind::WhatIf { min, max, step, .. } => {
            let step = if *step > 0.0 { *step } else { 1.0 };
            let mut rows = Vec::new();
            let mut v = *min;
            while v <= *max + step * 1e-9 && rows.len() < 100_000 {
                rows.push(vec![Scalar::Num((v * 1e9).round() / 1e9)]);
                v += step;
            }
            Ok(TableData { name: name.into(), columns: vec![name.into()], types: vec![ColType::Number], rows })
        }
        ParameterKind::Fields { items } => Ok(TableData {
            name: name.into(),
            columns: vec![name.into(), format!("{name} Order")],
            types: vec![ColType::Text, ColType::Integer],
            rows: items.iter().enumerate().map(|(i, f)| vec![Scalar::Text(f.label()), Scalar::Num(i as f64)]).collect(),
        }),
        ParameterKind::Value { .. } => crate::err("Value parameters have no table."),
    }
}

/// Replaces `@Name` parameter references in DAX with the parameters' current values.
pub fn substitute_params(dax: &str, p: &Project, overrides: &BTreeMap<String, Scalar>) -> String {
    if !dax.contains('@') {
        return dax.to_string();
    }
    let mut out = String::with_capacity(dax.len());
    let chars: Vec<char> = dax.chars().collect();
    let mut i = 0;
    let mut in_str = false;
    while i < chars.len() {
        let c = chars.get(i).copied().unwrap_or(' ');
        if c == '"' {
            in_str = !in_str;
        }
        if c == '@' && !in_str {
            // @Name or @[Name with spaces]
            let (name, next) = if chars.get(i + 1) == Some(&'[') {
                let end = chars.iter().skip(i + 2).position(|c| *c == ']').map(|e| e + i + 2).unwrap_or(chars.len());
                (chars.get(i + 2..end).map(|s| s.iter().collect::<String>()).unwrap_or_default(), end + 1)
            } else {
                let end = chars.iter().skip(i + 1).position(|c| !(c.is_alphanumeric() || *c == '_')).map(|e| e + i + 1).unwrap_or(chars.len());
                (chars.get(i + 1..end).map(|s| s.iter().collect::<String>()).unwrap_or_default(), end)
            };
            if let Some(v) = overrides.get(&name).cloned().or_else(|| p.parameter(&name).map(|x| x.current.clone())) {
                out.push_str(&v.dax());
                i = next;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

fn to_dm(v: &Scalar) -> DmValue {
    v.to_dm()
}

/// Builds the DAX model from loaded table data.
pub fn build_model(p: &Project, tables: &BTreeMap<String, Rc<TableData>>, params: &BTreeMap<String, Scalar>) -> (Model, Vec<(String, String)>) {
    let mut m = Model::new();
    let mut errors = Vec::new();
    for t in &p.model.tables {
        let Some(d) = tables.get(&t.name) else { continue };
        let mut headers = d.columns.clone();
        let mut rows: Vec<Vec<DmValue>> = d.rows.iter().map(|r| r.iter().map(to_dm).collect()).collect();
        if p.model.auto_date_time {
            for (ci, ty) in d.types.iter().enumerate() {
                if !ty.is_temporal() {
                    continue;
                }
                let col = d.columns.get(ci).cloned().unwrap_or_default();
                for lvl in DATE_LEVELS.iter().chain(["MonthNo"].iter()) {
                    headers.push(date_level_column(&col, lvl));
                }
                for (ri, row) in rows.iter_mut().enumerate() {
                    let serial = d.rows.get(ri).and_then(|r| r.get(ci)).and_then(|v| v.as_f64());
                    let parts = serial.and_then(|s| vizcraft_datamodel::dates::to_date(s.floor()));
                    match parts {
                        Some(date) => {
                            use chrono::Datelike;
                            row.push(DmValue::Number(date.year() as f64));
                            row.push(DmValue::text(format!("Qtr {}", date.month0() / 3 + 1)));
                            row.push(DmValue::text(crate::numfmt::format_date(serial.unwrap_or(0.0), "mmmm")));
                            row.push(DmValue::Number(date.day() as f64));
                            row.push(DmValue::Number(date.month() as f64));
                        }
                        None => row.extend(std::iter::repeat_n(DmValue::Blank, 5)),
                    }
                }
            }
        }
        match DmTable::from_rows(t.name.clone(), &headers, &rows).and_then(|tb| m.add_table(tb)) {
            Ok(_) => {}
            Err(e) => errors.push((t.name.clone(), e.to_string())),
        }
    }
    // Calculated tables.
    for t in &p.model.tables {
        if let TableSource::Calculated(dax) = &t.source {
            match m.query(&substitute_params(dax, p, params)) {
                Ok(res) => {
                    let headers: Vec<String> = res.columns.iter().map(|c| c.rsplit_once('[').map(|(_, r)| r.trim_end_matches(']').to_string()).unwrap_or(c.clone())).collect();
                    match DmTable::from_rows(t.name.clone(), &headers, &res.rows).and_then(|tb| m.add_table(tb)) {
                        Ok(_) => {}
                        Err(e) => errors.push((t.name.clone(), e.to_string())),
                    }
                }
                Err(e) => errors.push((t.name.clone(), e.to_string())),
            }
        }
    }
    for r in &p.model.relationships {
        let (from, to) = match r.cardinality {
            Cardinality::OneToMany => (((&r.to_table, &r.to_column)), (&r.from_table, &r.from_column)),
            _ => ((&r.from_table, &r.from_column), (&r.to_table, &r.to_column)),
        };
        if r.cardinality == Cardinality::ManyToMany {
            errors.push((format!("{}–{}", r.from_table, r.to_table), "Many-to-many relationships are kept but not used for filtering yet.".into()));
            continue;
        }
        let cf = if r.cross_filter == CrossFilterDir::Both { CrossFilter::Both } else { CrossFilter::Single };
        if let Err(e) = m.add_relationship((from.0, from.1), (to.0, to.1), cf, r.active) {
            errors.push((format!("{}[{}] → {}[{}]", r.from_table, r.from_column, r.to_table, r.to_column), e.to_string()));
        }
    }
    for c in &p.model.calculated_columns {
        if let Err(e) = m.add_calculated_column(&c.table, &c.name, &substitute_params(&c.expression, p, params)) {
            errors.push((format!("{}[{}]", c.table, c.name), e.to_string()));
        }
    }
    // Parameter measures.
    for prm in &p.model.parameters {
        if let ParameterKind::WhatIf { default, .. } = prm.kind {
            let q = vizcraft_datamodel::model::quote_table(&prm.name);
            let e = format!("SELECTEDVALUE({q}[{}], {default})", prm.name);
            if let Err(err) = m.add_measure(&prm.name, &format!("{} Value", prm.name), &e, None) {
                errors.push((prm.name.clone(), err.to_string()));
            }
        }
    }
    // Measures may refer to measures defined later: add in passes.
    let mut pending: Vec<&Measure> = p.model.measures.iter().collect();
    for _ in 0..8 {
        if pending.is_empty() {
            break;
        }
        let mut next = Vec::new();
        let mut last_err = Vec::new();
        for ms in pending {
            let fmt = if ms.format.is_empty() { None } else { Some(ms.format.clone()) };
            match m.add_measure(&ms.table, &ms.name, &substitute_params(&ms.expression, p, params), fmt) {
                Ok(()) => {}
                Err(e) => {
                    last_err.push((format!("[{}]", ms.name), e.to_string()));
                    next.push(ms);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        pending = next;
        if pending.len() == last_err.len() {
            // No progress possible after a pass where everything failed again.
            errors.extend(last_err);
            break;
        }
    }
    (m, errors)
}

/// Full refresh.
pub fn build(p: &Project, host: &dyn Host, params: &BTreeMap<String, Scalar>) -> Built {
    let (tables, mut errors) = load_tables(p, host);
    let (model, e2) = build_model(p, &tables, params);
    errors.extend(e2);
    Built { model: Rc::new(model), tables, errors }
}

/// Rebuilds the model from already loaded data (after a model edit or a parameter change).
pub fn rebuild(p: &Project, prev: &Built, params: &BTreeMap<String, Scalar>) -> Built {
    // Parameter tables may have changed shape.
    let mut tables = prev.tables.clone();
    for t in &p.model.tables {
        if let TableSource::Parameter(n) = &t.source
            && let Ok(d) = parameter_table(p, n)
        {
            tables.insert(t.name.clone(), Rc::new(d));
        }
        if let TableSource::Sheet(s) = &t.source
            && let Some(sh) = p.sheet(s)
        {
            let mut d = crate::sheet::evaluate_with(sh, &p.sheets);
            d.name = t.name.clone();
            d.normalize();
            apply_declared_types(&mut d, t);
            tables.insert(t.name.clone(), Rc::new(d));
        }
    }
    tables.retain(|k, _| p.model.tables.iter().any(|t| &t.name == k));
    let (model, errors) = build_model(p, &tables, params);
    Built { model: Rc::new(model), tables, errors }
}

/// Columns of a table as the field list shows them (not hidden, no auto date helpers), with
/// the type.
pub fn visible_columns(p: &Project, built: &Built, table: &str) -> Vec<(String, ColType)> {
    let mt = p.model.tables.iter().find(|t| t.name.eq_ignore_ascii_case(table));
    let mut out = Vec::new();
    if let Some(d) = built.table(table) {
        for (i, c) in d.columns.iter().enumerate() {
            let hidden = mt.and_then(|t| t.columns.iter().find(|cp| &cp.name == c)).is_some_and(|cp| cp.hidden);
            if !hidden {
                out.push((c.clone(), d.types.get(i).copied().unwrap_or_default()));
            }
        }
    } else if let Some(t) = built.model.table(table) {
        for c in &t.columns {
            if !c.name.contains('.') || !DATE_LEVELS.iter().chain(["MonthNo"].iter()).any(|l| c.name.ends_with(&format!(".{l}"))) {
                out.push((c.name.clone(), ColType::Any));
            }
        }
    }
    for c in p.model.calculated_columns.iter().filter(|c| c.table.eq_ignore_ascii_case(table)) {
        if !out.iter().any(|(n, _)| n == &c.name) {
            out.push((c.name.clone(), ColType::Any));
        }
    }
    out
}
