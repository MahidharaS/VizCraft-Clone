//! TMDL (Tabular Model Definition Language) view: the semantic model as an editable script.
//!
//! [`write`] produces the whole model: tables (columns with their properties, calculated columns,
//! measures, hierarchies, partitions with their Power Query M or DAX source), relationships, roles
//! and expressions (Power Query parameters and connection-only queries). [`apply`] parses an edited
//! script back (indentation-based, tabs or four spaces) and applies the differences: measures,
//! calculated columns, column and table properties, relationships, roles, M partitions and
//! parameters are created, updated and deleted. A script that starts with `createOrReplace` (as
//! [`write_object`] produces for one object) only creates and updates.

use std::collections::BTreeMap;

use crate::build::{Built, ColType};
use crate::doc::*;

// ------------------------------------------------------------------------------------- writing

/// A name as TMDL writes it: bare when it is an identifier, else in single quotes.
pub fn quote(name: &str) -> String {
    let ident = name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    if ident { name.to_string() } else { format!("'{}'", name.replace('\'', "''")) }
}

fn tabs(n: usize) -> String {
    "\t".repeat(n)
}

fn desc(out: &mut String, d: &str, ind: usize) {
    for line in d.lines().filter(|l| !l.trim().is_empty()) {
        out.push_str(&format!("{}/// {}\n", tabs(ind), line.trim()));
    }
}

/// `= expr` for a one-line expression, or a fenced block for a multi-line one.
fn expr_text(e: &str, ind: usize) -> String {
    let e = e.trim();
    if !e.contains('\n') {
        return format!(" = {e}");
    }
    let mut s = " = ```\n".to_string();
    for line in e.lines() {
        if line.trim().is_empty() {
            s.push('\n');
        } else {
            s.push_str(&format!("{}{}\n", tabs(ind + 2), line));
        }
    }
    s.push_str(&format!("{}```", tabs(ind + 2)));
    s
}

/// An indented multi-line source (`source =` of a partition, an M expression).
fn block(out: &mut String, e: &str, ind: usize) {
    for line in e.trim_end().lines() {
        if line.trim().is_empty() {
            out.push('\n');
        } else {
            out.push_str(&format!("{}{}\n", tabs(ind), line));
        }
    }
}

pub fn tmdl_type(t: ColType) -> &'static str {
    match t {
        ColType::Text | ColType::Any => "string",
        ColType::Integer => "int64",
        ColType::Number => "double",
        ColType::Date | ColType::DateTime | ColType::Time => "dateTime",
        ColType::Bool => "boolean",
    }
}

/// The Table view data type label for a TMDL `dataType`.
pub fn type_label(t: &str) -> &'static str {
    match t.to_ascii_lowercase().as_str() {
        "int64" => "Whole Number",
        "double" => "Decimal Number",
        "decimal" => "Fixed Decimal",
        "datetime" => "Date",
        "boolean" => "True/False",
        _ => "Text",
    }
}

fn summarize_word(s: &str) -> &'static str {
    match s {
        "Sum" => "sum",
        "Average" => "average",
        "Min" => "min",
        "Max" => "max",
        "Count" => "count",
        "Distinct count" => "distinctCount",
        _ => "none",
    }
}

fn summarize_label(s: &str) -> &'static str {
    match s.to_ascii_lowercase().as_str() {
        "sum" => "Sum",
        "average" => "Average",
        "min" => "Min",
        "max" => "Max",
        "count" => "Count",
        "distinctcount" => "Distinct count",
        _ => "None",
    }
}

/// A column's effective properties (what the script shows).
#[derive(Clone, Debug, Default, PartialEq)]
struct ColInfo {
    data_type: String,
    format: String,
    summarize: String,
    sort_by: String,
    hidden: bool,
    folder: String,
    category: String,
    description: String,
}

fn col_type(built: Option<&Built>, table: &str, column: &str) -> ColType {
    let Some(b) = built else { return ColType::Any };
    let t = b.col_type(table, column);
    if t != ColType::Any {
        return t;
    }
    // Calculated columns / tables: from the model's data.
    match b.model.table(table).and_then(|t| t.column_index(column).and_then(|i| t.column(i))).map(|c| c.data_type()) {
        Some(vizardcraft_datamodel::store::DataType::Number) => {
            let all_int = b
                .model
                .table(table)
                .and_then(|t| t.column_index(column).and_then(|i| t.column(i)))
                .is_some_and(|c| (0..c.cardinality() as u32).all(|id| c.dict_value(id).as_number().is_none_or(|x| x.fract() == 0.0)));
            if all_int { ColType::Integer } else { ColType::Number }
        }
        Some(vizardcraft_datamodel::store::DataType::Boolean) => ColType::Bool,
        Some(vizardcraft_datamodel::store::DataType::Text) => ColType::Text,
        _ => ColType::Any,
    }
}

fn col_info(p: &Project, built: Option<&Built>, table: &str, column: &str) -> ColInfo {
    let cp = p.model.tables.iter().find(|t| t.name.eq_ignore_ascii_case(table)).and_then(|t| t.columns.iter().find(|c| c.name.eq_ignore_ascii_case(column)));
    let ty = match cp.map(|c| c.data_type.as_str()).filter(|s| !s.is_empty()) {
        Some(label) => ColType::from_label(label),
        None => col_type(built, table, column),
    };
    let summarize = match cp.map(|c| c.summarize_by.as_str()).filter(|s| !s.is_empty()) {
        Some(s) => summarize_word(s).to_string(),
        None if ty.is_numeric() && !column.to_ascii_lowercase().ends_with("id") && !column.to_ascii_lowercase().contains("year") => "sum".into(),
        None => "none".into(),
    };
    ColInfo {
        data_type: tmdl_type(ty).into(),
        format: cp.map(|c| c.format.clone()).filter(|f| !f.is_empty()).unwrap_or_else(|| ty.default_format().to_string()),
        summarize,
        sort_by: cp.and_then(|c| c.sort_by.clone()).unwrap_or_default(),
        hidden: cp.is_some_and(|c| c.hidden),
        folder: cp.map(|c| c.display_folder.clone()).unwrap_or_default(),
        category: cp.map(|c| c.data_category.clone()).filter(|c| c != "Uncategorized").unwrap_or_default(),
        description: cp.map(|c| c.description.clone()).unwrap_or_default(),
    }
}

/// The columns of a model table: loaded columns, then calculated columns.
fn table_columns(p: &Project, built: &Built, table: &str) -> Vec<String> {
    let mut out: Vec<String> = match built.table(table) {
        Some(d) => d.columns.clone(),
        None => built
            .model
            .table(table)
            .map(|t| {
                t.columns
                    .iter()
                    .filter(|c| c.expression.is_none() && !crate::build::DATE_LEVELS.iter().chain(["MonthNo"].iter()).any(|l| c.name.ends_with(&format!(".{l}"))))
                    .map(|c| c.name.clone())
                    .collect()
            })
            .unwrap_or_default(),
    };
    // Columns with metadata but no data yet (before a refresh).
    if let Some(mt) = p.model.tables.iter().find(|t| t.name.eq_ignore_ascii_case(table)) {
        for c in &mt.columns {
            let calc = p.model.calculated_columns.iter().any(|cc| cc.table.eq_ignore_ascii_case(table) && cc.name.eq_ignore_ascii_case(&c.name));
            if !calc && !out.iter().any(|x| x.eq_ignore_ascii_case(&c.name)) && built.tables.is_empty() {
                out.push(c.name.clone());
            }
        }
    }
    out
}

fn write_column(out: &mut String, p: &Project, built: &Built, table: &str, column: &str, calc: Option<&str>, ind: usize) {
    let info = col_info(p, Some(built), table, column);
    desc(out, &info.description, ind);
    out.push_str(&format!("{}column {}{}\n", tabs(ind), quote(column), calc.map(|e| expr_text(e, ind)).unwrap_or_default()));
    let pr = |out: &mut String, k: &str, v: &str| {
        if !v.is_empty() {
            out.push_str(&format!("{}{k}: {v}\n", tabs(ind + 1)));
        }
    };
    pr(out, "dataType", &info.data_type);
    pr(out, "formatString", &info.format);
    pr(out, "summarizeBy", &info.summarize);
    if !info.sort_by.is_empty() {
        pr(out, "sortByColumn", &quote(&info.sort_by));
    }
    if info.hidden {
        out.push_str(&format!("{}isHidden\n", tabs(ind + 1)));
    }
    pr(out, "displayFolder", &info.folder);
    pr(out, "dataCategory", &info.category);
    if calc.is_none() {
        pr(out, "sourceColumn", column);
    }
    out.push('\n');
}

fn write_measure(out: &mut String, m: &Measure, ind: usize) {
    desc(out, &m.description, ind);
    out.push_str(&format!("{}measure {}{}\n", tabs(ind), quote(&m.name), expr_text(&m.expression, ind)));
    if !m.format.is_empty() {
        out.push_str(&format!("{}formatString: {}\n", tabs(ind + 1), m.format));
    }
    if !m.display_folder.is_empty() {
        out.push_str(&format!("{}displayFolder: {}\n", tabs(ind + 1), m.display_folder));
    }
    if m.hidden {
        out.push_str(&format!("{}isHidden\n", tabs(ind + 1)));
    }
    out.push('\n');
}

fn write_partition(out: &mut String, p: &Project, t: &ModelTable, ind: usize) {
    let mode = if t.storage_mode.eq_ignore_ascii_case("DirectQuery") { "directQuery" } else { "import" };
    match &t.source {
        TableSource::Query(q) => {
            out.push_str(&format!("{}partition {} = m\n", tabs(ind), quote(&t.name)));
            out.push_str(&format!("{}mode: {mode}\n", tabs(ind + 1)));
            out.push_str(&format!("{}source =\n", tabs(ind + 1)));
            block(out, p.queries.query(q).map(|q| q.m.as_str()).unwrap_or(""), ind + 3);
        }
        TableSource::Calculated(dax) => {
            out.push_str(&format!("{}partition {} = calculated\n", tabs(ind), quote(&t.name)));
            out.push_str(&format!("{}mode: import\n", tabs(ind + 1)));
            out.push_str(&format!("{}source =\n", tabs(ind + 1)));
            block(out, dax, ind + 3);
        }
        TableSource::Parameter(name) => {
            out.push_str(&format!("{}partition {} = calculated\n", tabs(ind), quote(&t.name)));
            out.push_str(&format!("{}mode: import\n", tabs(ind + 1)));
            let src = match p.parameter(name).map(|x| &x.kind) {
                Some(ParameterKind::WhatIf { min, max, step, .. }) => format!("GENERATESERIES({min}, {max}, {step})"),
                Some(ParameterKind::Fields { items }) => format!(
                    "{{{}}}",
                    items.iter().enumerate().map(|(i, f)| format!("(\"{}\", NAMEOF({}), {i})", f.label().replace('"', "\"\""), field_dax(f))).collect::<Vec<_>>().join(", ")
                ),
                _ => String::new(),
            };
            out.push_str(&format!("{}source = {src}\n", tabs(ind + 1)));
            out.push_str(&format!("{}annotation VizardCraft.Parameter = {}\n", tabs(ind + 1), quote(name)));
        }
        TableSource::Sheet(s) => {
            out.push_str(&format!("{}partition {} = entity\n", tabs(ind), quote(&t.name)));
            out.push_str(&format!("{}mode: import\n", tabs(ind + 1)));
            out.push_str(&format!("{}source: dataSheet {}\n", tabs(ind + 1), quote(s)));
        }
        TableSource::Flow(spec) => {
            out.push_str(&format!("{}partition {} = entity\n", tabs(ind), quote(&t.name)));
            out.push_str(&format!("{}mode: import\n", tabs(ind + 1)));
            out.push_str(&format!("{}source: flow {}\n", tabs(ind + 1), quote(spec)));
        }
        TableSource::None => {}
    }
    out.push('\n');
}

fn field_dax(f: &FieldRef) -> String {
    let q = vizardcraft_datamodel::model::quote_table;
    match f {
        FieldRef::Column { table, column } => format!("{}[{column}]", q(table)),
        FieldRef::Measure { name, .. } => format!("[{name}]"),
        FieldRef::Level { table, hierarchy, level } => format!("{}[{hierarchy}.{level}]", q(table)),
        FieldRef::FieldParam { name } => format!("[{name}]"),
    }
}

fn write_table(out: &mut String, p: &Project, built: &Built, t: &ModelTable, ind: usize, only_measure: Option<&str>) {
    desc(out, &t.description, ind);
    out.push_str(&format!("{}table {}\n", tabs(ind), quote(&t.name)));
    if only_measure.is_none() {
        if t.hidden {
            out.push_str(&format!("{}isHidden\n", tabs(ind + 1)));
        }
        if t.is_date_table {
            out.push_str(&format!("{}dataCategory: Time\n", tabs(ind + 1)));
        }
    }
    out.push('\n');
    for m in p.model.measures.iter().filter(|m| m.table.eq_ignore_ascii_case(&t.name)) {
        if only_measure.is_none_or(|n| n.eq_ignore_ascii_case(&m.name)) {
            write_measure(out, m, ind + 1);
        }
    }
    if only_measure.is_some() {
        return;
    }
    for c in table_columns(p, built, &t.name) {
        if p.model.calculated_columns.iter().any(|cc| cc.table.eq_ignore_ascii_case(&t.name) && cc.name.eq_ignore_ascii_case(&c)) {
            continue;
        }
        write_column(out, p, built, &t.name, &c, None, ind + 1);
    }
    for cc in p.model.calculated_columns.iter().filter(|c| c.table.eq_ignore_ascii_case(&t.name)) {
        write_column(out, p, built, &t.name, &cc.name, Some(&cc.expression), ind + 1);
    }
    for h in &t.hierarchies {
        out.push_str(&format!("{}hierarchy {}\n", tabs(ind + 1), quote(&h.name)));
        for l in &h.levels {
            out.push_str(&format!("{}level {}\n", tabs(ind + 2), quote(l)));
            out.push_str(&format!("{}column: {}\n", tabs(ind + 3), quote(l)));
        }
        out.push('\n');
    }
    write_partition(out, p, t, ind + 1);
}

fn rel_end(table: &str, column: &str) -> String {
    format!("{}.{}", quote(table), quote(column))
}

fn write_relationship(out: &mut String, r: &Relationship, ind: usize) {
    out.push_str(&format!("{}relationship {}\n", tabs(ind), r.id));
    let pr = |out: &mut String, k: &str, v: &str| out.push_str(&format!("{}{k}: {v}\n", tabs(ind + 1)));
    pr(out, "fromColumn", &rel_end(&r.from_table, &r.from_column));
    pr(out, "toColumn", &rel_end(&r.to_table, &r.to_column));
    let (from, to) = match r.cardinality {
        Cardinality::ManyToOne => ("many", "one"),
        Cardinality::OneToOne => ("one", "one"),
        Cardinality::OneToMany => ("one", "many"),
        Cardinality::ManyToMany => ("many", "many"),
    };
    if from != "many" {
        pr(out, "fromCardinality", from);
    }
    pr(out, "toCardinality", to);
    if r.cross_filter == CrossFilterDir::Both {
        pr(out, "crossFilteringBehavior", "bothDirections");
    }
    if !r.active {
        pr(out, "isActive", "false");
    }
    if r.security_both {
        pr(out, "securityFilteringBehavior", "bothDirections");
    }
    if r.assume_integrity {
        out.push_str(&format!("{}relyOnReferentialIntegrity\n", tabs(ind + 1)));
    }
    out.push('\n');
}

fn write_role(out: &mut String, r: &Role, ind: usize) {
    out.push_str(&format!("{}role {}\n", tabs(ind), quote(&r.name)));
    out.push_str(&format!("{}modelPermission: read\n", tabs(ind + 1)));
    for m in &r.members {
        out.push_str(&format!("{}member {}\n", tabs(ind + 1), quote(m)));
    }
    for (t, f) in &r.filters {
        out.push_str(&format!("{}tablePermission {}{}\n", tabs(ind + 1), quote(t), expr_text(f, ind + 1)));
    }
    out.push('\n');
}

fn m_type(kind: &str) -> &'static str {
    match kind {
        "Decimal Number" | "Whole Number" | "Number" | "Fixed Decimal" => "Number",
        "Date" => "Date",
        "Date/Time" => "DateTime",
        "True/False" | "Logical" => "Logical",
        "Any" => "Any",
        _ => "Text",
    }
}

fn write_expressions(out: &mut String, p: &Project, ind: usize) {
    for prm in &p.queries.parameters {
        desc(out, &prm.description, ind);
        out.push_str(&format!(
            "{}expression {} = {} meta [IsParameterQuery=true, Type=\"{}\", IsParameterQueryRequired=true]\n\n",
            tabs(ind),
            quote(&prm.name),
            prm.value.trim(),
            m_type(&prm.kind)
        ));
    }
    for q in p.queries.queries.iter().filter(|q| q.load == vizardcraft_query::steps::LoadTo::None) {
        desc(out, &q.description, ind);
        out.push_str(&format!("{}expression {} =\n", tabs(ind), quote(&q.name)));
        block(out, &q.m, ind + 2);
        out.push('\n');
    }
    // Tableau-style value parameters, referenced in DAX as @Name.
    for prm in &p.model.parameters {
        if let ParameterKind::Value { data_type, .. } = &prm.kind {
            desc(out, &prm.description, ind);
            out.push_str(&format!(
                "{}expression {} = {} meta [IsParameterQuery=true, Type=\"{}\", Scope=\"Model\"]\n\n",
                tabs(ind),
                quote(&prm.name),
                m_literal(&prm.current),
                m_type(data_type)
            ));
        }
    }
}

fn m_literal(s: &crate::Scalar) -> String {
    match s {
        crate::Scalar::Null => "null".into(),
        crate::Scalar::Bool(b) => b.to_string(),
        crate::Scalar::Num(n) => crate::numfmt::general(*n),
        crate::Scalar::Text(t) => format!("\"{}\"", t.replace('"', "\"\"")),
    }
}

/// Writes the whole model as TMDL.
pub fn write(p: &Project, built: &Built) -> String {
    let mut s = String::new();
    s.push_str(&format!("model Model\n\tculture: {}\n", if p.model.culture.is_empty() { "en-US" } else { &p.model.culture }));
    s.push_str("\tdefaultPowerBIDataSourceVersion: powerBI_V3\n");
    if p.model.auto_date_time {
        s.push_str("\tautoDateTime\n");
    }
    s.push('\n');
    for t in &p.model.tables {
        write_table(&mut s, p, built, t, 0, None);
    }
    for r in &p.model.relationships {
        write_relationship(&mut s, r, 0);
    }
    for r in &p.model.roles {
        write_role(&mut s, r, 0);
    }
    write_expressions(&mut s, p, 0);
    while s.ends_with("\n\n") {
        s.pop();
    }
    s
}

/// One model object, for "Script TMDL" (a `createOrReplace` script).
#[derive(Clone, Debug, PartialEq)]
pub enum TmdlObject {
    Table(String),
    Measure(String),
    Relationship(u64),
    Role(String),
    Expressions,
}

/// A `createOrReplace` script for one object.
pub fn write_object(p: &Project, built: &Built, obj: &TmdlObject) -> String {
    let mut s = "createOrReplace\n\n".to_string();
    match obj {
        TmdlObject::Table(name) => {
            if let Some(t) = p.model.tables.iter().find(|t| t.name.eq_ignore_ascii_case(name)) {
                write_table(&mut s, p, built, t, 1, None);
            }
        }
        TmdlObject::Measure(name) => {
            if let Some(m) = p.model.measures.iter().find(|m| m.name.eq_ignore_ascii_case(name))
                && let Some(t) = p.model.tables.iter().find(|t| t.name.eq_ignore_ascii_case(&m.table))
            {
                write_table(&mut s, p, built, t, 1, Some(&m.name));
            }
        }
        TmdlObject::Relationship(id) => {
            if let Some(r) = p.model.relationships.iter().find(|r| r.id == *id) {
                write_relationship(&mut s, r, 1);
            }
        }
        TmdlObject::Role(name) => {
            if let Some(r) = p.model.roles.iter().find(|r| r.name.eq_ignore_ascii_case(name)) {
                write_role(&mut s, r, 1);
            }
        }
        TmdlObject::Expressions => write_expressions(&mut s, p, 1),
    }
    while s.ends_with("\n\n") {
        s.pop();
    }
    s
}

// ------------------------------------------------------------------------------------- parsing

/// One object of a script: `kind name [= expression]`, its properties and child objects.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    pub kind: String,
    pub name: String,
    pub expr: Option<String>,
    /// Properties in order: `name: value`, flags (`isHidden`) as "true", `source = …` expressions.
    pub props: Vec<(String, String)>,
    pub children: Vec<Node>,
    pub description: String,
    /// 1-based line number.
    pub line: usize,
}

impl Node {
    pub fn prop(&self, k: &str) -> Option<&str> {
        self.props.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str())
    }
    fn flag(&self, k: &str) -> bool {
        self.prop(k).is_some_and(|v| !v.eq_ignore_ascii_case("false"))
    }
    fn kids<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.kind.eq_ignore_ascii_case(kind))
    }
}

const OBJECTS: &[&str] = &[
    "model",
    "table",
    "measure",
    "column",
    "hierarchy",
    "level",
    "partition",
    "relationship",
    "role",
    "tablepermission",
    "member",
    "expression",
    "annotation",
    "ref",
    "culture",
    "perspective",
    "datasource",
    "calculationgroup",
    "calculationitem",
    "variation",
    "changedproperty",
];

fn indent_of(line: &str) -> usize {
    let mut n = 0;
    let mut spaces = 0;
    for c in line.chars() {
        match c {
            '\t' => {
                n += 1;
                spaces = 0;
            }
            ' ' => {
                spaces += 1;
                if spaces == 4 {
                    n += 1;
                    spaces = 0;
                }
            }
            _ => break,
        }
    }
    n
}

/// Strips `n` levels of indentation (tabs, or four spaces each).
fn dedent(line: &str, n: usize) -> &str {
    let mut rest = line;
    for _ in 0..n {
        if let Some(r) = rest.strip_prefix('\t') {
            rest = r;
        } else if let Some(r) = rest.strip_prefix("    ") {
            rest = r;
        } else {
            break;
        }
    }
    rest
}

/// `'quoted name' rest` or `bare name = rest` → (name, rest after the name).
fn split_name(s: &str) -> (String, &str) {
    let s = s.trim_start();
    if let Some(body) = s.strip_prefix('\'') {
        let mut name = String::new();
        let mut chars = body.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '\'' {
                if chars.peek().map(|(_, c)| *c) == Some('\'') {
                    name.push('\'');
                    chars.next();
                } else {
                    return (name, body.get(i + 1..).unwrap_or(""));
                }
            } else {
                name.push(c);
            }
        }
        return (name, "");
    }
    // A bare name ends at ` =`, `=` or the end of the line; `.` separates in references.
    let end = s.find('=').unwrap_or(s.len());
    let name = s.get(..end).unwrap_or(s).trim_end();
    (name.to_string(), s.get(end..).unwrap_or(""))
}

/// `Table.Column` with optional quotes.
pub fn split_ref(s: &str) -> Option<(String, String)> {
    let s = s.trim();
    if s.starts_with('\'') {
        let (t, rest) = split_name(s);
        let rest = rest.trim_start().strip_prefix('.')?;
        let (c, _) = split_name(rest);
        return Some((t, c));
    }
    let (t, c) = s.split_once('.')?;
    let c = c.trim();
    let c = if c.starts_with('\'') { split_name(c).0 } else { c.to_string() };
    Some((t.trim().to_string(), c))
}

struct Lines<'a> {
    lines: Vec<&'a str>,
    i: usize,
}

impl Lines<'_> {
    /// Lines deeper than `level` (or blank), dedented by `level`, as one text: an expression's
    /// continuation.
    fn deeper(&mut self, level: usize) -> String {
        let mut out: Vec<&str> = Vec::new();
        while let Some(l) = self.lines.get(self.i) {
            if l.trim().is_empty() {
                // A blank line belongs to the expression when a deeper line follows.
                let next = self.lines.iter().skip(self.i + 1).find(|l| !l.trim().is_empty());
                if next.is_some_and(|n| indent_of(n) >= level) {
                    out.push("");
                    self.i += 1;
                    continue;
                }
                break;
            }
            if indent_of(l) < level {
                break;
            }
            out.push(dedent(l, level));
            self.i += 1;
        }
        while out.last().is_some_and(|l| l.trim().is_empty()) {
            out.pop();
        }
        out.join("\n")
    }

    /// A fenced block after "```" up to the closing fence, with common indentation removed.
    fn fenced(&mut self, start_line: usize) -> Result<String, String> {
        let mut body: Vec<&str> = Vec::new();
        loop {
            let Some(l) = self.lines.get(self.i) else { return Err(format!("Line {start_line}: the ``` block isn't closed.")) };
            self.i += 1;
            if l.trim() == "```" {
                break;
            }
            body.push(l);
        }
        let common = body.iter().filter(|l| !l.trim().is_empty()).map(|l| indent_of(l)).min().unwrap_or(0);
        Ok(body.iter().map(|l| dedent(l, common)).collect::<Vec<_>>().join("\n").trim_end().to_string())
    }

    fn block(&mut self, level: usize) -> Result<Vec<Node>, String> {
        let mut out = Vec::new();
        let mut pending_desc: Vec<String> = Vec::new();
        while let Some(raw) = self.lines.get(self.i).copied() {
            let line_no = self.i + 1;
            let t = raw.trim();
            if t.is_empty() {
                self.i += 1;
                continue;
            }
            let ind = indent_of(raw);
            if ind < level {
                break;
            }
            if ind > level {
                return Err(format!("Line {line_no}: unexpected indentation (expected {level} tab{}).", if level == 1 { "" } else { "s" }));
            }
            self.i += 1;
            if let Some(d) = t.strip_prefix("///") {
                pending_desc.push(d.trim().to_string());
                continue;
            }
            if t.starts_with("//") {
                continue;
            }
            let word_end = t.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')).unwrap_or(t.len());
            let word = t.get(..word_end).unwrap_or(t);
            let rest = t.get(word_end..).unwrap_or("");
            if !OBJECTS.contains(&word.to_ascii_lowercase().as_str()) || rest.trim_start().starts_with(':') {
                return Err(format!("Line {line_no}: `{t}` is a property, but no object owns it here."));
            }
            let (name, after) = split_name(rest);
            let mut node = Node { kind: word.to_ascii_lowercase(), name, line: line_no, description: pending_desc.join("\n"), ..Node::default() };
            pending_desc.clear();
            if let Some(e) = after.trim_start().strip_prefix('=') {
                let e = e.trim();
                node.expr = Some(if e == "```" {
                    self.fenced(line_no)?
                } else {
                    let more = self.deeper(level + 2);
                    match (e.is_empty(), more.is_empty()) {
                        (true, _) => more,
                        (false, true) => e.to_string(),
                        (false, false) => format!("{e}\n{more}"),
                    }
                });
            }
            self.props(&mut node, level + 1)?;
            out.push(node);
        }
        Ok(out)
    }

    /// Properties and child objects at `level`.
    fn props(&mut self, node: &mut Node, level: usize) -> Result<(), String> {
        loop {
            // Skip blank lines (but stop at a shallower line).
            while self.lines.get(self.i).is_some_and(|l| l.trim().is_empty()) {
                self.i += 1;
            }
            let Some(raw) = self.lines.get(self.i).copied() else { return Ok(()) };
            let ind = indent_of(raw);
            if ind < level {
                return Ok(());
            }
            let line_no = self.i + 1;
            if ind > level {
                return Err(format!("Line {line_no}: unexpected indentation."));
            }
            let t = raw.trim();
            if t.starts_with("//") && !t.starts_with("///") {
                self.i += 1;
                continue;
            }
            let word_end = t.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')).unwrap_or(t.len());
            let word = t.get(..word_end).unwrap_or(t);
            let rest = t.get(word_end..).unwrap_or("").trim_start();
            if t.starts_with("///") || (OBJECTS.contains(&word.to_ascii_lowercase().as_str()) && !rest.starts_with(':')) {
                let mut kids = self.block(level)?;
                node.children.append(&mut kids);
                continue;
            }
            self.i += 1;
            if let Some(v) = rest.strip_prefix(':') {
                node.props.push((word.to_string(), v.trim().to_string()));
            } else if let Some(e) = rest.strip_prefix('=') {
                let e = e.trim();
                let v = if e == "```" {
                    self.fenced(line_no)?
                } else {
                    let more = self.deeper(level + 2);
                    if e.is_empty() { more } else if more.is_empty() { e.to_string() } else { format!("{e}\n{more}") }
                };
                node.props.push((word.to_string(), v));
            } else if rest.is_empty() {
                node.props.push((word.to_string(), "true".into()));
            } else {
                return Err(format!("Line {line_no}: can't read `{t}` (expected `property: value`)."));
            }
        }
    }
}

/// Parses a script into its top-level objects; `true` when it is a `createOrReplace` script.
pub fn parse(script: &str) -> Result<(Vec<Node>, bool), String> {
    let lines: Vec<&str> = script.lines().collect();
    let first = lines.iter().position(|l| {
        let t = l.trim();
        !t.is_empty() && !(t.starts_with("//") && !t.starts_with("///"))
    });
    let partial = first.and_then(|i| lines.get(i)).is_some_and(|l| l.trim().eq_ignore_ascii_case("createOrReplace"));
    let mut ls = Lines { lines, i: 0 };
    if partial {
        ls.i = first.map(|i| i + 1).unwrap_or(0);
        let nodes = ls.block(1)?;
        if let Some(l) = ls.lines.get(ls.i) {
            return Err(format!("Line {}: `{}` should be indented under createOrReplace.", ls.i + 1, l.trim()));
        }
        Ok((nodes, true))
    } else {
        let nodes = ls.block(0)?;
        Ok((nodes, false))
    }
}

// ------------------------------------------------------------------------------------- applying

/// Applies an edited TMDL script to the project and returns what changed. Nothing changes when
/// the script has an error.
pub fn apply(p: &mut Project, script: &str) -> Result<Vec<String>, String> {
    apply_with(p, None, script)
}

/// [`apply`], with the loaded data to tell default column properties from edited ones.
pub fn apply_with(p: &mut Project, built: Option<&Built>, script: &str) -> Result<Vec<String>, String> {
    let (nodes, partial) = parse(script)?;
    let mut q = p.clone();
    let mut log = Vec::new();
    let mut errors = Vec::new();
    let full = !partial;

    for n in nodes.iter().filter(|n| n.kind == "model") {
        if let Some(c) = n.prop("culture")
            && c != q.model.culture
            && !(q.model.culture.is_empty() && c == "en-US")
        {
            q.model.culture = c.to_string();
            log.push(format!("Model culture → {c}"));
        }
        let auto = n.flag("autoDateTime");
        if full && auto != q.model.auto_date_time {
            q.model.auto_date_time = auto;
            log.push(format!("Auto date/time {}", if auto { "on" } else { "off" }));
        }
    }

    // Tables.
    let tables: Vec<&Node> = nodes.iter().filter(|n| n.kind == "table").collect();
    for tn in &tables {
        apply_table(&mut q, built, tn, partial, &mut log, &mut errors);
    }
    if full {
        // Measures whose table section is gone are deleted with it; calculated tables too.
        let names: Vec<String> = tables.iter().map(|t| t.name.to_lowercase()).collect();
        let gone: Vec<String> = q.model.tables.iter().filter(|t| matches!(t.source, TableSource::Calculated(_)) && !names.contains(&t.name.to_lowercase())).map(|t| t.name.clone()).collect();
        for g in gone {
            q.model.tables.retain(|t| t.name != g);
            q.model.measures.retain(|m| !m.table.eq_ignore_ascii_case(&g));
            q.model.calculated_columns.retain(|c| !c.table.eq_ignore_ascii_case(&g));
            log.push(format!("Deleted calculated table '{g}'"));
        }
        let declared: Vec<(String, String)> = tables.iter().flat_map(|t| t.kids("measure").map(|m| (m.name.to_lowercase(), t.name.to_lowercase()))).collect();
        let before = q.model.measures.len();
        let mut removed = Vec::new();
        q.model.measures.retain(|m| {
            let keep = declared.iter().any(|(n, _)| *n == m.name.to_lowercase());
            if !keep {
                removed.push(m.name.clone());
            }
            keep
        });
        if q.model.measures.len() != before {
            for r in removed {
                log.push(format!("Deleted measure [{r}]"));
            }
        }
    }

    // Relationships.
    let rels: Vec<&Node> = nodes.iter().filter(|n| n.kind == "relationship").collect();
    let mut seen_rel = Vec::new();
    for rn in &rels {
        let (Some(from), Some(to)) = (rn.prop("fromColumn").and_then(split_ref), rn.prop("toColumn").and_then(split_ref)) else {
            errors.push(format!("Line {}: a relationship needs fromColumn and toColumn.", rn.line));
            continue;
        };
        for (t, c) in [&from, &to] {
            if !q.model.tables.iter().any(|x| x.name.eq_ignore_ascii_case(t)) {
                errors.push(format!("Line {}: there's no table '{t}' (in {t}.{c}).", rn.line));
            }
        }
        let fc = rn.prop("fromCardinality").unwrap_or("many").to_ascii_lowercase();
        let tc = rn.prop("toCardinality").unwrap_or("one").to_ascii_lowercase();
        let card = match (fc.as_str(), tc.as_str()) {
            ("one", "one") => Cardinality::OneToOne,
            ("one", "many") => Cardinality::OneToMany,
            ("many", "many") => Cardinality::ManyToMany,
            _ => Cardinality::ManyToOne,
        };
        let cross = if rn.prop("crossFilteringBehavior").is_some_and(|v| v.eq_ignore_ascii_case("bothDirections")) { CrossFilterDir::Both } else { CrossFilterDir::Single };
        let active = !rn.prop("isActive").is_some_and(|v| v.eq_ignore_ascii_case("false"));
        let sec = rn.prop("securityFilteringBehavior").is_some_and(|v| v.eq_ignore_ascii_case("bothDirections"));
        let rely = rn.flag("relyOnReferentialIntegrity");
        let id_num: Option<u64> = rn.name.trim().parse().ok();
        let idx = id_num.and_then(|id| q.model.relationships.iter().position(|r| r.id == id)).or_else(|| {
            q.model.relationships.iter().position(|r| {
                r.from_table.eq_ignore_ascii_case(&from.0) && r.from_column.eq_ignore_ascii_case(&from.1) && r.to_table.eq_ignore_ascii_case(&to.0) && r.to_column.eq_ignore_ascii_case(&to.1)
            })
        });
        let new = Relationship {
            id: 0,
            from_table: from.0.clone(),
            from_column: from.1.clone(),
            to_table: to.0.clone(),
            to_column: to.1.clone(),
            cardinality: card,
            cross_filter: cross,
            active,
            security_both: sec,
            assume_integrity: rely,
        };
        let label = format!("{}[{}] → {}[{}]", from.0, from.1, to.0, to.1);
        match idx.and_then(|i| q.model.relationships.get_mut(i)) {
            Some(r) => {
                let id = r.id;
                let candidate = Relationship { id, ..new };
                if *r != candidate {
                    *r = candidate;
                    log.push(format!("Updated relationship {label}"));
                }
                seen_rel.push(id);
            }
            None => {
                let id = id_num.filter(|id| !q.model.relationships.iter().any(|r| r.id == *id)).unwrap_or_else(|| q.next_id());
                q.model.relationships.push(Relationship { id, ..new });
                seen_rel.push(id);
                log.push(format!("Created relationship {label}"));
            }
        }
    }
    if full {
        let mut removed = Vec::new();
        q.model.relationships.retain(|r| {
            let keep = seen_rel.contains(&r.id);
            if !keep {
                removed.push(format!("Deleted relationship {}[{}] → {}[{}]", r.from_table, r.from_column, r.to_table, r.to_column));
            }
            keep
        });
        log.extend(removed);
    }

    // Roles.
    let roles: Vec<&Node> = nodes.iter().filter(|n| n.kind == "role").collect();
    for rn in &roles {
        let mut filters = Vec::new();
        for tp in rn.kids("tablepermission") {
            let f = tp.expr.clone().unwrap_or_default();
            if !f.trim().is_empty()
                && let Err(e) = vizardcraft_datamodel::dax::parse(&f)
            {
                errors.push(format!("Line {}: role '{}', table {}: {e}", tp.line, rn.name, tp.name));
            }
            filters.push((tp.name.clone(), f));
        }
        let members: Vec<String> = rn.kids("member").map(|m| m.name.clone()).collect();
        match q.model.roles.iter_mut().find(|r| r.name.eq_ignore_ascii_case(&rn.name)) {
            Some(r) => {
                if r.filters != filters || r.members != members {
                    r.filters = filters;
                    r.members = members;
                    log.push(format!("Updated role '{}'", rn.name));
                }
            }
            None => {
                q.model.roles.push(Role { name: rn.name.clone(), filters, members });
                log.push(format!("Created role '{}'", rn.name));
            }
        }
    }
    if full {
        let names: Vec<String> = roles.iter().map(|r| r.name.to_lowercase()).collect();
        let before: Vec<String> = q.model.roles.iter().map(|r| r.name.clone()).collect();
        q.model.roles.retain(|r| names.contains(&r.name.to_lowercase()));
        for b in before.iter().filter(|b| !names.contains(&b.to_lowercase())) {
            log.push(format!("Deleted role '{b}'"));
        }
    }

    // Expressions: Power Query parameters, connection-only queries, value parameters.
    for en in nodes.iter().filter(|n| n.kind == "expression") {
        let text = en.expr.clone().unwrap_or_default();
        let (value, meta) = match text.find(" meta [") {
            Some(i) => (text.get(..i).unwrap_or("").trim().to_string(), Some(text.get(i..).unwrap_or(""))),
            None => (text.trim().to_string(), None),
        };
        let model_scope = meta.is_some_and(|m| m.contains("Scope=\"Model\""));
        if model_scope {
            if let Some(prm) = q.model.parameters.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&en.name)) {
                let v = parse_m_literal(&value);
                if prm.current != v {
                    prm.current = v;
                    log.push(format!("Parameter {} = {value}", en.name));
                }
            } else {
                errors.push(format!("Line {}: there's no parameter '{}'.", en.line, en.name));
            }
        } else if meta.is_some_and(|m| m.contains("IsParameterQuery=true")) {
            let kind = meta.and_then(|m| m.split("Type=\"").nth(1)).and_then(|r| r.split('"').next()).unwrap_or("Text");
            let kind_label = match kind {
                "Number" => "Decimal Number",
                "Date" => "Date",
                "DateTime" => "Date/Time",
                "Logical" => "True/False",
                "Any" => "Any",
                _ => "Text",
            };
            match q.queries.parameters.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&en.name)) {
                Some(prm) => {
                    if prm.value.trim() != value {
                        prm.value = value.clone();
                        log.push(format!("Power Query parameter {} = {value}", en.name));
                    }
                }
                None => {
                    q.queries.parameters.push(vizardcraft_query::steps::Parameter {
                        name: en.name.clone(),
                        kind: kind_label.into(),
                        value: value.clone(),
                        description: en.description.clone(),
                        suggestions: vec![],
                    });
                    log.push(format!("Created Power Query parameter {}", en.name));
                }
            }
        } else {
            match q.queries.queries.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&en.name)) {
                Some(qd) => {
                    if qd.m.trim() != text.trim() {
                        qd.m = text.trim().to_string();
                        log.push(format!("Updated query {}", en.name));
                    }
                }
                None => {
                    q.queries.queries.push(vizardcraft_query::steps::QueryDef {
                        name: en.name.clone(),
                        m: text.trim().to_string(),
                        load: vizardcraft_query::steps::LoadTo::None,
                        loaded_table: None,
                        group: None,
                        description: en.description.clone(),
                    });
                    log.push(format!("Created query {} (connection only)", en.name));
                }
            }
        }
    }

    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    *p = q;
    Ok(log)
}

fn parse_m_literal(v: &str) -> crate::Scalar {
    let t = v.trim();
    if t.eq_ignore_ascii_case("null") {
        return crate::Scalar::Null;
    }
    if t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("false") {
        return crate::Scalar::Bool(t.eq_ignore_ascii_case("true"));
    }
    if let Some(s) = t.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return crate::Scalar::Text(s.replace("\"\"", "\""));
    }
    t.parse::<f64>().map(crate::Scalar::Num).unwrap_or_else(|_| crate::Scalar::Text(t.to_string()))
}

fn set<T: PartialEq + Clone>(slot: &mut T, v: T, log: &mut Vec<String>, msg: impl FnOnce() -> String) {
    if *slot != v {
        *slot = v;
        log.push(msg());
    }
}

fn apply_table(q: &mut Project, built: Option<&Built>, tn: &Node, partial: bool, log: &mut Vec<String>, errors: &mut Vec<String>) {
    let tname = tn.name.clone();
    let partition = tn.kids("partition").next();
    let exists = q.model.tables.iter().any(|t| t.name.eq_ignore_ascii_case(&tname));
    if !exists {
        match partition.map(|pn| (pn.expr.as_deref().unwrap_or("").trim().to_ascii_lowercase(), pn.prop("source").unwrap_or("").to_string())) {
            Some((kind, src)) if kind == "calculated" && !src.trim().is_empty() => {
                if let Err(e) = vizardcraft_datamodel::dax::parse(&src) {
                    errors.push(format!("Line {}: table '{tname}': {e}", tn.line));
                    return;
                }
                q.model.tables.push(ModelTable { name: tname.clone(), source: TableSource::Calculated(src.trim().to_string()), storage_mode: "Import".into(), ..ModelTable::default() });
                log.push(format!("Created calculated table '{tname}'"));
            }
            Some((kind, src)) if kind == "m" && !src.trim().is_empty() => {
                q.queries.queries.push(vizardcraft_query::steps::QueryDef {
                    name: tname.clone(),
                    m: src.trim().to_string(),
                    load: vizardcraft_query::steps::LoadTo::Model,
                    loaded_table: None,
                    group: None,
                    description: tn.description.clone(),
                });
                q.model.tables.push(ModelTable { name: tname.clone(), source: TableSource::Query(tname.clone()), storage_mode: "Import".into(), ..ModelTable::default() });
                log.push(format!("Created table '{tname}' from Power Query"));
            }
            _ => {
                errors.push(format!("Line {}: there's no table '{tname}' (a new table needs a `partition … = calculated` or `= m` with a source).", tn.line));
                return;
            }
        }
    }
    let Some(ti) = q.model.tables.iter().position(|t| t.name.eq_ignore_ascii_case(&tname)) else { return };
    let tname = q.model.tables.get(ti).map(|t| t.name.clone()).unwrap_or(tname);

    // Table properties.
    if let Some(t) = q.model.tables.get_mut(ti) {
        if !partial || tn.prop("isHidden").is_some() {
            set(&mut t.hidden, tn.flag("isHidden"), log, || format!("Table '{tname}': hidden = {}", tn.flag("isHidden")));
        }
        if !partial || tn.prop("dataCategory").is_some() {
            let dt = tn.prop("dataCategory").is_some_and(|v| v.eq_ignore_ascii_case("Time"));
            set(&mut t.is_date_table, dt, log, || format!("Table '{tname}': {} date table", if dt { "marked as" } else { "no longer a" }));
        }
        if !partial || !tn.description.is_empty() {
            set(&mut t.description, tn.description.clone(), log, || format!("Table '{tname}': description"));
        }
    }

    // Partition source.
    if let Some(pn) = partition {
        let kind = pn.expr.as_deref().unwrap_or("").trim().to_ascii_lowercase();
        let src = pn.prop("source").unwrap_or("").trim().to_string();
        let mode = pn.prop("mode").unwrap_or("import");
        let source = q.model.tables.get(ti).map(|t| t.source.clone()).unwrap_or_default();
        match (&source, kind.as_str()) {
            (TableSource::Query(qn), "m") => {
                if let Some(qd) = q.queries.queries.iter_mut().find(|x| &x.name == qn)
                    && qd.m.trim() != src
                    && !src.is_empty()
                {
                    qd.m = src.clone();
                    log.push(format!("Table '{tname}': Power Query source updated (refresh to load)"));
                }
            }
            (TableSource::Calculated(old), "calculated") => {
                if old.trim() != src && !src.is_empty() {
                    match vizardcraft_datamodel::dax::parse(&src) {
                        Ok(_) => {
                            if let Some(t) = q.model.tables.get_mut(ti) {
                                t.source = TableSource::Calculated(src.clone());
                            }
                            log.push(format!("Table '{tname}': calculated table expression updated"));
                        }
                        Err(e) => errors.push(format!("Line {}: table '{tname}': {e}", pn.line)),
                    }
                }
            }
            _ => {}
        }
        if let Some(t) = q.model.tables.get_mut(ti) {
            let sm = if mode.eq_ignore_ascii_case("directQuery") { "DirectQuery" } else { "Import" };
            if !(t.storage_mode.is_empty() && sm == "Import") {
                set(&mut t.storage_mode, sm.to_string(), log, || format!("Table '{tname}': storage mode {sm}"));
            }
        }
    }

    // Measures.
    for mn in tn.kids("measure") {
        let expr = mn.expr.clone().unwrap_or_default();
        if let Err(e) = vizardcraft_datamodel::dax::parse(&expr) {
            errors.push(format!("Line {}: measure [{}]: {e}", mn.line, mn.name));
            continue;
        }
        let new = Measure {
            table: tname.clone(),
            name: mn.name.clone(),
            expression: expr.trim().to_string(),
            format: mn.prop("formatString").unwrap_or("").to_string(),
            description: mn.description.clone(),
            display_folder: mn.prop("displayFolder").unwrap_or("").to_string(),
            hidden: mn.flag("isHidden"),
            origin: String::new(),
        };
        if q.model.tables.iter().any(|t| t.name.eq_ignore_ascii_case(&tname) && t.columns.iter().any(|c| c.name.eq_ignore_ascii_case(&mn.name)))
            && built.is_some_and(|b| b.table(&tname).is_some_and(|d| d.column_index(&mn.name).is_some()))
        {
            errors.push(format!("Line {}: [{}] is already a column of '{tname}'.", mn.line, mn.name));
            continue;
        }
        match q.model.measures.iter_mut().find(|m| m.name.eq_ignore_ascii_case(&mn.name)) {
            Some(m) => {
                let candidate = Measure { origin: m.origin.clone(), ..new };
                if *m != candidate {
                    let what = if m.table != candidate.table {
                        format!("moved to '{tname}'")
                    } else if m.expression != candidate.expression {
                        "expression".to_string()
                    } else {
                        "properties".to_string()
                    };
                    *m = candidate;
                    log.push(format!("Updated measure [{}] ({what})", mn.name));
                }
            }
            None => {
                q.model.measures.push(new);
                log.push(format!("Created measure [{}]", mn.name));
            }
        }
    }

    // Columns.
    let mut calc_seen = Vec::new();
    for cn in tn.kids("column") {
        if let Some(expr) = &cn.expr {
            if let Err(e) = vizardcraft_datamodel::dax::parse(expr) {
                errors.push(format!("Line {}: column [{}]: {e}", cn.line, cn.name));
                continue;
            }
            calc_seen.push(cn.name.to_lowercase());
            match q.model.calculated_columns.iter_mut().find(|c| c.table.eq_ignore_ascii_case(&tname) && c.name.eq_ignore_ascii_case(&cn.name)) {
                Some(c) => {
                    if c.expression.trim() != expr.trim() {
                        c.expression = expr.trim().to_string();
                        log.push(format!("Updated calculated column '{tname}'[{}]", cn.name));
                    }
                }
                None => {
                    q.model.calculated_columns.push(CalcColumn { table: tname.clone(), name: cn.name.clone(), expression: expr.trim().to_string(), origin: String::new() });
                    log.push(format!("Created calculated column '{tname}'[{}]", cn.name));
                }
            }
        }
        apply_column_props(q, built, &tname, ti, cn, partial, log);
    }
    if !partial {
        let mut removed = Vec::new();
        q.model.calculated_columns.retain(|c| {
            let keep = !c.table.eq_ignore_ascii_case(&tname) || calc_seen.contains(&c.name.to_lowercase());
            if !keep {
                removed.push(format!("Deleted calculated column '{tname}'[{}]", c.name));
            }
            keep
        });
        log.extend(removed);
    }

    // Hierarchies.
    let hs: Vec<Hierarchy> = tn
        .kids("hierarchy")
        .map(|h| Hierarchy { name: h.name.clone(), levels: h.kids("level").map(|l| l.prop("column").map(|c| split_name(c).0).filter(|c| !c.is_empty()).unwrap_or_else(|| l.name.clone())).collect() })
        .collect();
    if let Some(t) = q.model.tables.get_mut(ti)
        && (!partial || !hs.is_empty())
        && t.hierarchies != hs
    {
        t.hierarchies = hs;
        log.push(format!("Table '{tname}': hierarchies updated"));
    }
}

fn apply_column_props(q: &mut Project, built: Option<&Built>, tname: &str, ti: usize, cn: &Node, partial: bool, log: &mut Vec<String>) {
    let mut current = col_info(q, built, tname, &cn.name);
    if built.is_none() {
        // Without the data, defaults aren't known: what the script says for properties the
        // project doesn't set is taken as the current value.
        let cp = q.model.tables.get(ti).and_then(|t| t.columns.iter().find(|c| c.name.eq_ignore_ascii_case(&cn.name)));
        if cp.is_none_or(|c| c.data_type.is_empty()) {
            current.data_type = cn.prop("dataType").map(str::to_string).unwrap_or(current.data_type);
        }
        if cp.is_none_or(|c| c.format.is_empty()) {
            current.format = cn.prop("formatString").map(str::to_string).unwrap_or(current.format);
        }
        if cp.is_none_or(|c| c.summarize_by.is_empty()) {
            current.summarize = cn.prop("summarizeBy").map(str::to_string).unwrap_or(current.summarize);
        }
    }
    let has = |k: &str| cn.prop(k).is_some();
    let want = ColInfo {
        data_type: cn.prop("dataType").map(str::to_string).unwrap_or_else(|| current.data_type.clone()),
        format: cn.prop("formatString").map(str::to_string).unwrap_or_else(|| if partial { current.format.clone() } else { String::new() }),
        summarize: cn.prop("summarizeBy").map(str::to_string).unwrap_or_else(|| current.summarize.clone()),
        sort_by: cn.prop("sortByColumn").map(|s| split_name(s).0).unwrap_or_else(|| if partial { current.sort_by.clone() } else { String::new() }),
        hidden: if partial && !has("isHidden") { current.hidden } else { cn.flag("isHidden") },
        folder: cn.prop("displayFolder").map(str::to_string).unwrap_or_else(|| if partial { current.folder.clone() } else { String::new() }),
        category: cn.prop("dataCategory").map(str::to_string).unwrap_or_else(|| if partial { current.category.clone() } else { String::new() }),
        description: if partial && cn.description.is_empty() { current.description.clone() } else { cn.description.clone() },
    };
    if want == current {
        return;
    }
    let Some(t) = q.model.tables.get_mut(ti) else { return };
    let idx = match t.columns.iter().position(|c| c.name.eq_ignore_ascii_case(&cn.name)) {
        Some(i) => i,
        None => {
            t.columns.push(ColumnProps { name: cn.name.clone(), ..ColumnProps::default() });
            t.columns.len() - 1
        }
    };
    let Some(cp) = t.columns.get_mut(idx) else { return };
    let mut changed: Vec<&str> = Vec::new();
    if want.data_type != current.data_type {
        cp.data_type = type_label(&want.data_type).into();
        changed.push("data type");
    }
    if want.format != current.format {
        cp.format = want.format.clone();
        changed.push("format");
    }
    if want.summarize != current.summarize {
        cp.summarize_by = summarize_label(&want.summarize).into();
        changed.push("summarization");
    }
    if want.sort_by != current.sort_by {
        cp.sort_by = Some(want.sort_by.clone()).filter(|s| !s.is_empty());
        changed.push("sort by column");
    }
    if want.hidden != current.hidden {
        cp.hidden = want.hidden;
        changed.push(if want.hidden { "hidden" } else { "shown" });
    }
    if want.folder != current.folder {
        cp.display_folder = want.folder.clone();
        changed.push("display folder");
    }
    if want.category != current.category {
        cp.data_category = want.category.clone();
        changed.push("data category");
    }
    if want.description != current.description {
        cp.description = want.description.clone();
        changed.push("description");
    }
    if !changed.is_empty() {
        log.push(format!("Column '{tname}'[{}]: {}", cn.name, changed.join(", ")));
    }
}

/// Lines that differ between two scripts, for a preview: (line in `new`, "+"/"-", text).
pub fn diff_lines(old: &str, new: &str) -> Vec<(usize, char, String)> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    // Longest common subsequence on lines (scripts are small).
    let (n, m) = (a.len(), b.len());
    if n.saturating_mul(m) > 4_000_000 {
        return vec![];
    }
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let v = if a.get(i) == b.get(j) {
                dp.get(i + 1).and_then(|r| r.get(j + 1)).copied().unwrap_or(0) + 1
            } else {
                dp.get(i + 1).and_then(|r| r.get(j)).copied().unwrap_or(0).max(dp.get(i).and_then(|r| r.get(j + 1)).copied().unwrap_or(0))
            };
            if let Some(slot) = dp.get_mut(i).and_then(|r| r.get_mut(j)) {
                *slot = v;
            }
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    let at = |i: usize, j: usize| dp.get(i).and_then(|r| r.get(j)).copied().unwrap_or(0);
    while i < n || j < m {
        if i < n && j < m && a.get(i) == b.get(j) {
            i += 1;
            j += 1;
        } else if j < m && (i >= n || at(i, j + 1) >= at(i + 1, j)) {
            out.push((j + 1, '+', b.get(j).map(|s| s.to_string()).unwrap_or_default()));
            j += 1;
        } else {
            out.push((j + 1, '-', a.get(i).map(|s| s.to_string()).unwrap_or_default()));
            i += 1;
        }
    }
    out
}

/// Table name → measure names, for completions.
pub fn outline(p: &Project) -> BTreeMap<String, Vec<String>> {
    let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for t in &p.model.tables {
        m.entry(t.name.clone()).or_default();
    }
    for ms in &p.model.measures {
        m.entry(ms.table.clone()).or_default().push(ms.name.clone());
    }
    m
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;

    fn sample() -> (Project, Built) {
        let p = crate::sample::sales();
        let b = crate::build::build(&p, &vizardcraft_query::NoHost, &Default::default());
        (p, b)
    }

    #[test]
    fn writes_the_whole_model() {
        let (p, b) = sample();
        let s = write(&p, &b);
        assert!(s.starts_with("model Model\n\tculture: en-US"));
        assert!(s.contains("table Sales\n"));
        assert!(s.contains("\tmeasure 'Total Sales' = SUMX(Sales, Sales[Quantity] * Sales[Unit Price] * (1 - Sales[Discount]))\n\t\tformatString: $#,0\n"));
        assert!(s.contains("\tcolumn 'Unit Price'\n\t\tdataType: double\n\t\tformatString: $#,0.00\n"));
        assert!(s.contains("\tpartition Product = m\n\t\tmode: import\n\t\tsource =\n\t\t\t\tlet\n"));
        assert!(s.contains("\thierarchy Date\n\t\tlevel Year\n\t\t\tcolumn: Year\n"));
        assert!(s.contains("relationship "));
        assert!(s.contains("\tfromColumn: Sales.ProductID\n\ttoColumn: Product.ProductID\n\ttoCardinality: one\n"));
        assert!(s.contains("partition 'Discount scenario' = calculated"));
    }

    #[test]
    fn round_trip_has_no_changes() {
        let (mut p, b) = sample();
        p.model.roles.push(Role { name: "East".into(), filters: vec![("Store".into(), "[Region] = \"East\"".into())], members: vec!["ann@contoso.com".into()] });
        p.model.measures.push(Measure { table: "Sales".into(), name: "Multi".into(), expression: "VAR x = [Units]\nRETURN\n    x * 2".into(), description: "Two lines\nof notes".into(), display_folder: "KPIs".into(), ..Measure::default() });
        p.model.calculated_columns.push(CalcColumn { table: "Sales".into(), name: "Line Total".into(), expression: "Sales[Quantity] * Sales[Unit Price]".into(), origin: String::new() });
        p.queries.parameters.push(vizardcraft_query::steps::Parameter { name: "Server".into(), kind: "Text".into(), value: "\"localhost\"".into(), description: String::new(), suggestions: vec![] });
        let b = crate::build::rebuild(&p, &b, &Default::default());
        let s = write(&p, &b);
        let before = p.clone();
        let log = apply_with(&mut p, Some(&b), &s).unwrap_or_else(|e| panic!("{e}\n{s}"));
        assert!(log.is_empty(), "{log:?}");
        assert_eq!(p, before);
        // Without the loaded data, too.
        let log = apply(&mut p, &s).unwrap();
        assert!(log.is_empty(), "{log:?}");
        assert!(s.contains("\tmeasure Multi = ```\n\t\t\tVAR x = [Units]\n\t\t\tRETURN\n\t\t\t    x * 2\n\t\t\t```\n"));
    }

    #[test]
    fn applies_edits() {
        let (mut p, b) = sample();
        let s = write(&p, &b);
        let s = s
            .replace("measure Units = SUM(Sales[Quantity])", "measure Units = SUM(Sales[Quantity]) + 0\n\t\tdisplayFolder: Volume")
            .replace("\tmeasure Orders = DISTINCTCOUNT(Sales[OrderID])\n\t\tformatString: #,0\n\n", "")
            .replace("\tcolumn 'Unit Price'\n\t\tdataType: double\n\t\tformatString: $#,0.00", "\tcolumn 'Unit Price'\n\t\tdataType: double\n\t\tformatString: 0.00\n\t\tisHidden")
            .replace("table Sales\n", "table Sales\n\n\t/// Average units per order line\n\tmeasure 'Avg Units' = AVERAGE(Sales[Quantity])\n\t\tformatString: 0.0\n\n\tcolumn Big = IF(Sales[Quantity] >= 3, \"Big\", \"Small\")\n\n")
            + "\n\nrelationship 99\n\tfromColumn: Targets.Region\n\ttoColumn: Store.Region\n\ttoCardinality: many\n\tisActive: false\n\nrole Managers\n\tmodelPermission: read\n\ttablePermission Store = [Region] = \"West\"\n";
        let log = apply_with(&mut p, Some(&b), &s).unwrap_or_else(|e| panic!("{e}"));
        let has = |x: &str| log.iter().any(|l| l.contains(x));
        assert!(has("Updated measure [Units]"), "{log:?}");
        assert!(has("Deleted measure [Orders]"), "{log:?}");
        assert!(has("Created measure [Avg Units]"), "{log:?}");
        assert!(has("Created calculated column 'Sales'[Big]"), "{log:?}");
        assert!(has("Column 'Sales'[Unit Price]: format, hidden"), "{log:?}");
        assert!(has("Created relationship Targets[Region] → Store[Region]"), "{log:?}");
        assert!(has("Created role 'Managers'"), "{log:?}");
        let m = p.model.measures.iter().find(|m| m.name == "Units").unwrap();
        assert_eq!((m.expression.as_str(), m.display_folder.as_str()), ("SUM(Sales[Quantity]) + 0", "Volume"));
        assert_eq!(p.model.measures.iter().find(|m| m.name == "Avg Units").unwrap().description, "Average units per order line");
        let r = p.model.relationships.iter().find(|r| r.id == 99).unwrap();
        assert_eq!((r.cardinality, r.active), (Cardinality::ManyToMany, false));
        // The edited model builds.
        let b2 = crate::build::rebuild(&p, &b, &Default::default());
        assert!(b2.model.measure("Avg Units").is_some());
        // Applying the new script again changes nothing.
        let s2 = write(&p, &b2);
        let log = apply_with(&mut p, Some(&b2), &s2).unwrap();
        assert!(log.is_empty(), "{log:?}");
    }

    #[test]
    fn errors_and_partial_scripts() {
        let (mut p, b) = sample();
        let before = p.clone();
        let bad = write(&p, &b).replace("measure Units = SUM(Sales[Quantity])", "measure Units = SUM(Sales[Quantity]");
        let e = apply_with(&mut p, Some(&b), &bad).unwrap_err();
        assert!(e.contains("measure [Units]"), "{e}");
        assert_eq!(p, before);
        assert!(apply(&mut p, "table Sales\n\t\tmeasure X = 1").unwrap_err().contains("Line 2"));
        // createOrReplace only adds/updates.
        let s = write_object(&p, &b, &TmdlObject::Measure("Profit".into()));
        assert!(s.starts_with("createOrReplace\n\n\ttable Sales\n"));
        let s = s.replace("[Total Sales] - [Total Cost]", "[Total Sales] - [Total Cost] - 1");
        let log = apply_with(&mut p, Some(&b), &s).unwrap();
        assert_eq!(log, vec!["Updated measure [Profit] (expression)".to_string()]);
        assert_eq!(p.model.measures.len(), before.model.measures.len());
        let t = write_object(&p, &b, &TmdlObject::Table("Product".into()));
        assert!(apply_with(&mut p, Some(&b), &t).unwrap().is_empty());
        let d = diff_lines("a\nb\nc", "a\nB\nc\nd");
        assert_eq!(d, vec![(2, '+', "B".into()), (3, '-', "b".into()), (4, '+', "d".into())]);
    }
}
