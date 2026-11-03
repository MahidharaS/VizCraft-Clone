//! Tableau-style calculated fields: translates Tableau calculation syntax (IF/THEN/ELSE/END,
//! IIF, CASE, ZN, LOD expressions `{FIXED …}`, DATETRUNC, table calculations, …) to DAX.
//!
//! A calculation whose result is aggregated (it uses SUM, AVG, COUNTD, a table calculation, an
//! INCLUDE/EXCLUDE LOD or a measure) becomes a DAX measure; one that works row by row (including
//! a FIXED LOD, which Tableau treats as a dimension) becomes a calculated column.

use crate::build::{Built, ColType};
use crate::doc::{Project, TableSource};
use vizcraft_datamodel::model::quote_table;

/// What a `[Field]` reference stands for.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    Column { table: String, column: String, text: bool },
    Measure(String),
    Parameter(String),
}

/// The result of a translation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Translation {
    pub dax: String,
    /// The result is aggregated: create a measure (else a calculated column).
    pub aggregated: bool,
    /// The table a calculated column belongs in (the table its fields come from).
    pub home_table: String,
    /// Approximations made (shown under the editor).
    pub warnings: Vec<String>,
}

/// Translates a Tableau calculation to DAX. `[Field]` references are columns of `default_table`
/// (`Table[Column]` and `[Table].[Column]` name a table). Text that is already DAX passes through.
pub fn to_dax(calc: &str, default_table: &str) -> Result<String, String> {
    let resolve = |name: &str, qual: Option<&str>| -> Option<FieldKind> {
        if qual.is_some_and(|q| q.eq_ignore_ascii_case("Parameters")) {
            return Some(FieldKind::Parameter(name.to_string()));
        }
        Some(FieldKind::Column { table: qual.unwrap_or(default_table).to_string(), column: name.to_string(), text: false })
    };
    translate(calc, default_table, &resolve, None).map(|t| t.dax)
}

/// Translates with fields resolved against a project's model: columns of any table (the default
/// table first), measures, parameters (`[Parameters].[Name]` or a bare parameter name).
pub fn translate_in(p: &Project, built: &Built, calc: &str, default_table: &str) -> Result<Translation, String> {
    let tables: Vec<String> = {
        let mut v: Vec<String> = p.model.tables.iter().map(|t| t.name.clone()).collect();
        if let Some(i) = v.iter().position(|t| t.eq_ignore_ascii_case(default_table)) {
            let d = v.remove(i);
            v.insert(0, d);
        }
        v
    };
    let columns_of = |t: &str| -> Vec<(String, bool)> {
        let mut out: Vec<(String, bool)> = match built.table(t) {
            Some(d) => d.columns.iter().enumerate().map(|(i, c)| (c.clone(), d.types.get(i) == Some(&ColType::Text))).collect(),
            None => built
                .model
                .table(t)
                .map(|mt| mt.columns.iter().map(|c| (c.name.clone(), c.data_type() == vizcraft_datamodel::store::DataType::Text)).collect())
                .unwrap_or_default(),
        };
        if let Some(mt) = built.model.table(t) {
            for c in &mt.columns {
                if c.expression.is_some() && !out.iter().any(|(n, _)| n == &c.name) {
                    out.push((c.name.clone(), c.data_type() == vizcraft_datamodel::store::DataType::Text));
                }
            }
        }
        out
    };
    let resolve = |name: &str, qual: Option<&str>| -> Option<FieldKind> {
        if let Some(q) = qual {
            if q.eq_ignore_ascii_case("Parameters") {
                return Some(FieldKind::Parameter(name.to_string()));
            }
            let t = tables.iter().find(|t| t.eq_ignore_ascii_case(q))?;
            let (c, text) = columns_of(t).into_iter().find(|(c, _)| c.eq_ignore_ascii_case(name))?;
            return Some(FieldKind::Column { table: t.clone(), column: c, text });
        }
        if let Some(m) = p.model.measures.iter().find(|m| m.name.eq_ignore_ascii_case(name)) {
            return Some(FieldKind::Measure(m.name.clone()));
        }
        for t in &tables {
            if let Some((c, text)) = columns_of(t).into_iter().find(|(c, _)| c.eq_ignore_ascii_case(name)) {
                return Some(FieldKind::Column { table: t.clone(), column: c, text });
            }
        }
        if let Some(prm) = p.parameter(name) {
            return Some(FieldKind::Parameter(prm.name.clone()));
        }
        // Parameter measures (`Discount scenario Value`).
        built.model.measure(name).map(|m| FieldKind::Measure(m.name.clone()))
    };
    // The date column table calculations order by: the marked date table's first date column,
    // else the first date column of the default table.
    let order = p
        .model
        .tables
        .iter()
        .filter(|t| t.is_date_table)
        .chain(p.model.tables.iter().filter(|t| t.name.eq_ignore_ascii_case(default_table)))
        .chain(p.model.tables.iter().filter(|t| !matches!(t.source, TableSource::Parameter(_))))
        .find_map(|t| {
            let d = built.table(&t.name)?;
            let i = d.types.iter().position(|ty| ty.is_temporal())?;
            Some((t.name.clone(), d.columns.get(i)?.clone()))
        });
    translate(calc, default_table, &resolve, order)
}

/// Translates a Tableau calculation. `resolve(name, qualifier)` says what `[name]` (or
/// `[qualifier].[name]` / `qualifier[name]`) is; `order` is the column table calculations
/// (RUNNING_SUM, WINDOW_AVG, RANK, …) run along.
pub fn translate(calc: &str, default_table: &str, resolve: &dyn Fn(&str, Option<&str>) -> Option<FieldKind>, order: Option<(String, String)>) -> Result<Translation, String> {
    let trimmed = calc.trim();
    if trimmed.is_empty() {
        return Err("The calculation is empty.".into());
    }
    let toks = match lex(trimmed) {
        Ok(t) => t,
        Err(e) => return passthrough(trimmed, default_table).ok_or(e),
    };
    let mut p = Parser { toks, pos: 0 };
    let ast = match p.expr(0).and_then(|e| if p.pos < p.toks.len() { Err(format!("Unexpected {} after the end of the calculation.", p.toks.get(p.pos).map(describe).unwrap_or_default())) } else { Ok(e) }) {
        Ok(a) => a,
        Err(e) => return passthrough(trimmed, default_table).ok_or(e),
    };
    let aggregated = is_aggregated(&ast, resolve);
    // Home table: the table most fields come from (the default table on a tie).
    let mut counts: Vec<(String, usize)> = Vec::new();
    collect_tables(&ast, resolve, &mut counts);
    let home = counts
        .iter()
        .max_by_key(|(t, n)| (*n, t.eq_ignore_ascii_case(default_table)))
        .map(|(t, _)| t.clone())
        .unwrap_or_else(|| default_table.to_string());
    let mut em = Emitter { resolve, warnings: vec![], order, vars: 0, default_table: home.clone() };
    let cx = if aggregated { Cx::Agg } else { Cx::Row(home.clone()) };
    let dax = em.emit(&ast, &cx)?;
    Ok(Translation { dax, aggregated, home_table: home, warnings: em.warnings })
}

/// DAX text: kept as it is when it parses as DAX.
fn passthrough(text: &str, default_table: &str) -> Option<Translation> {
    vizcraft_datamodel::dax::parse(text).ok()?;
    let upper = text.to_ascii_uppercase();
    let aggregated = ["SUM", "AVERAGE", "COUNT", "MIN(", "MAX(", "CALCULATE", "DISTINCTCOUNT", "DIVIDE(["].iter().any(|f| upper.contains(f));
    Some(Translation { dax: text.to_string(), aggregated, home_table: default_table.to_string(), warnings: vec!["Kept as DAX.".into()] })
}

// ------------------------------------------------------------------------------------- lexing

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Date(String),
    Field(String),
    Ident(String),
    Op(String),
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Dot,
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Num(n) => format!("number {n}"),
        Tok::Str(s) => format!("'{s}'"),
        Tok::Date(s) => format!("#{s}#"),
        Tok::Field(s) => format!("[{s}]"),
        Tok::Ident(s) => s.clone(),
        Tok::Op(o) => format!("`{o}`"),
        Tok::LParen => "`(`".into(),
        Tok::RParen => "`)`".into(),
        Tok::LBrace => "`{`".into(),
        Tok::RBrace => "`}`".into(),
        Tok::Comma => "`,`".into(),
        Tok::Colon => "`:`".into(),
        Tok::Dot => "`.`".into(),
    }
}

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = s.chars().collect();
    let at = |i: usize| c.get(i).copied();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(ch) = at(i) {
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch == '/' && at(i + 1) == Some('/') {
            while at(i).is_some_and(|x| x != '\n') {
                i += 1;
            }
            continue;
        }
        if ch == '/' && at(i + 1) == Some('*') {
            i += 2;
            while at(i).is_some() && !(at(i) == Some('*') && at(i + 1) == Some('/')) {
                i += 1;
            }
            i += 2;
            continue;
        }
        match ch {
            '0'..='9' => {
                let st = i;
                while at(i).is_some_and(|d| d.is_ascii_digit() || d == '.') {
                    i += 1;
                }
                if at(i).is_some_and(|e| e == 'e' || e == 'E') && at(i + 1).is_some_and(|d| d.is_ascii_digit() || d == '-' || d == '+') {
                    i += 2;
                    while at(i).is_some_and(|d| d.is_ascii_digit()) {
                        i += 1;
                    }
                }
                let t: String = c.get(st..i).unwrap_or_default().iter().collect();
                out.push(Tok::Num(t.parse().map_err(|_| format!("`{t}` is not a number."))?));
            }
            '.' if at(i + 1).is_some_and(|d| d.is_ascii_digit()) => {
                let st = i;
                i += 1;
                while at(i).is_some_and(|d| d.is_ascii_digit()) {
                    i += 1;
                }
                let t: String = c.get(st..i).unwrap_or_default().iter().collect();
                out.push(Tok::Num(t.parse().map_err(|_| format!("`{t}` is not a number."))?));
            }
            '\'' | '"' => {
                let (t, n) = delimited(&c, i, ch)?;
                out.push(Tok::Str(t));
                i = n;
            }
            '#' => {
                let (t, n) = delimited(&c, i, '#')?;
                out.push(Tok::Date(t));
                i = n;
            }
            '[' => {
                let (t, n) = delimited(&c, i, ']')?;
                out.push(Tok::Field(t));
                i = n;
            }
            '(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            '{' => {
                out.push(Tok::LBrace);
                i += 1;
            }
            '}' => {
                out.push(Tok::RBrace);
                i += 1;
            }
            ',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            ':' => {
                out.push(Tok::Colon);
                i += 1;
            }
            '.' => {
                out.push(Tok::Dot);
                i += 1;
            }
            _ if ch.is_alphabetic() || ch == '_' => {
                let st = i;
                while at(i).is_some_and(|d| d.is_alphanumeric() || d == '_') {
                    i += 1;
                }
                out.push(Tok::Ident(c.get(st..i).unwrap_or_default().iter().collect()));
            }
            _ => {
                let two: String = [ch, at(i + 1).unwrap_or(' ')].iter().collect();
                if ["<=", ">=", "<>", "!=", "==", "&&", "||"].contains(&two.as_str()) {
                    out.push(Tok::Op(match two.as_str() {
                        "&&" => "AND".into(),
                        "||" => "OR".into(),
                        other => other.into(),
                    }));
                    i += 2;
                    continue;
                }
                if "+-*/%^=<>".contains(ch) {
                    out.push(Tok::Op(ch.to_string()));
                    i += 1;
                } else {
                    return Err(format!("Unexpected character `{ch}`."));
                }
            }
        }
    }
    Ok(out)
}

fn delimited(c: &[char], i: usize, close: char) -> Result<(String, usize), String> {
    let mut j = i + 1;
    let mut s = String::new();
    loop {
        match c.get(j) {
            None => return Err(format!("Missing closing `{close}`.")),
            Some(&x) if x == close => {
                if c.get(j + 1) == Some(&close) {
                    s.push(close);
                    j += 2;
                } else {
                    return Ok((s, j + 1));
                }
            }
            Some(&x) => {
                s.push(x);
                j += 1;
            }
        }
    }
}

// ------------------------------------------------------------------------------------- parsing

#[derive(Clone, Debug, PartialEq)]
enum E {
    Num(f64),
    Str(String),
    Bool(bool),
    Null,
    Date(String),
    Field(String, Option<String>),
    Neg(Box<E>),
    Not(Box<E>),
    Bin(String, Box<E>, Box<E>),
    Call(String, Vec<E>),
    If(Vec<(E, E)>, Option<Box<E>>),
    Case(Box<E>, Vec<(E, E)>, Option<Box<E>>),
    /// `{FIXED|INCLUDE|EXCLUDE dims : expr}`; kind "" for `{ expr }`.
    Lod(String, Vec<E>, Box<E>),
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

fn prec(op: &str) -> u8 {
    match op {
        "OR" => 1,
        "AND" => 2,
        "=" | "==" | "<>" | "!=" | "<" | ">" | "<=" | ">=" => 4,
        "+" | "-" => 5,
        "*" | "/" | "%" => 6,
        "^" => 7,
        _ => 0,
    }
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }
    fn kw(&mut self, k: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Ident(s)) if s.eq_ignore_ascii_case(k)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn is_kw(&self, k: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(s)) if s.eq_ignore_ascii_case(k))
    }
    fn expect(&mut self, t: Tok) -> Result<(), String> {
        match self.next() {
            Some(x) if x == t => Ok(()),
            Some(x) => Err(format!("Expected {} but found {}.", describe(&t), describe(&x))),
            None => Err(format!("Expected {} but the calculation ended.", describe(&t))),
        }
    }
    fn expect_kw(&mut self, k: &str) -> Result<(), String> {
        if self.kw(k) {
            Ok(())
        } else {
            Err(format!("Expected {k}{}.", self.peek().map(|t| format!(" but found {}", describe(t))).unwrap_or_else(|| " but the calculation ended".into())))
        }
    }

    fn infix(&self) -> Option<String> {
        match self.peek() {
            Some(Tok::Op(o)) => Some(o.clone()),
            Some(Tok::Ident(k)) if k.eq_ignore_ascii_case("AND") => Some("AND".into()),
            Some(Tok::Ident(k)) if k.eq_ignore_ascii_case("OR") => Some("OR".into()),
            _ => None,
        }
    }

    fn expr(&mut self, min: u8) -> Result<E, String> {
        let mut lhs = self.prefix()?;
        while let Some(op) = self.infix() {
            let p = prec(&op);
            if p == 0 || p < min {
                break;
            }
            self.pos += 1;
            let rhs = if op == "^" { self.expr(p)? } else { self.expr(p + 1)? };
            lhs = E::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn prefix(&mut self) -> Result<E, String> {
        let Some(t) = self.next() else { return Err("The calculation ended early.".into()) };
        Ok(match t {
            Tok::Num(n) => E::Num(n),
            Tok::Str(s) => E::Str(s),
            Tok::Date(d) => E::Date(d),
            Tok::Op(o) if o == "-" => E::Neg(Box::new(self.expr(8)?)),
            Tok::Op(o) if o == "+" => self.expr(8)?,
            Tok::LParen => {
                let e = self.expr(0)?;
                self.expect(Tok::RParen)?;
                e
            }
            Tok::LBrace => self.lod()?,
            Tok::Field(f) => {
                // `[Table].[Column]` / `[Parameters].[Name]`.
                if self.peek() == Some(&Tok::Dot) && matches!(self.toks.get(self.pos + 1), Some(Tok::Field(_))) {
                    self.pos += 1;
                    match self.next() {
                        Some(Tok::Field(c)) => E::Field(c, Some(f)),
                        _ => E::Field(f, None),
                    }
                } else {
                    E::Field(f, None)
                }
            }
            Tok::Ident(name) => self.ident(name)?,
            other => return Err(format!("Unexpected {}.", describe(&other))),
        })
    }

    fn ident(&mut self, name: String) -> Result<E, String> {
        let up = name.to_ascii_uppercase();
        match up.as_str() {
            "IF" => return self.if_expr(),
            "CASE" => return self.case_expr(),
            "NOT" => return Ok(E::Not(Box::new(self.expr(3)?))),
            "TRUE" if self.peek() != Some(&Tok::LParen) => return Ok(E::Bool(true)),
            "FALSE" if self.peek() != Some(&Tok::LParen) => return Ok(E::Bool(false)),
            "NULL" => return Ok(E::Null),
            _ => {}
        }
        match self.peek() {
            Some(Tok::LParen) => {
                self.pos += 1;
                let mut args = Vec::new();
                if self.peek() != Some(&Tok::RParen) {
                    loop {
                        args.push(self.expr(0)?);
                        if self.peek() == Some(&Tok::Comma) {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                }
                self.expect(Tok::RParen)?;
                Ok(E::Call(up, args))
            }
            // DAX-style `Table[Column]`.
            Some(Tok::Field(_)) => match self.next() {
                Some(Tok::Field(c)) => Ok(E::Field(c, Some(name))),
                _ => Err("Expected a field.".into()),
            },
            _ => Err(format!("Unknown name `{name}` (fields go in brackets: [{name}]).")),
        }
    }

    fn if_expr(&mut self) -> Result<E, String> {
        let mut branches = Vec::new();
        loop {
            let c = self.expr(0)?;
            self.expect_kw("THEN")?;
            let v = self.expr(0)?;
            branches.push((c, v));
            if self.kw("ELSEIF") {
                continue;
            }
            // `ELSE IF` written as two words opens a nested IF … END.
            break;
        }
        let els = if self.kw("ELSE") { Some(Box::new(self.expr(0)?)) } else { None };
        self.expect_kw("END")?;
        Ok(E::If(branches, els))
    }

    fn case_expr(&mut self) -> Result<E, String> {
        let subject = self.expr(0)?;
        let mut whens = Vec::new();
        while self.kw("WHEN") {
            let v = self.expr(0)?;
            self.expect_kw("THEN")?;
            whens.push((v, self.expr(0)?));
        }
        if whens.is_empty() {
            return Err("CASE needs at least one WHEN … THEN.".into());
        }
        let els = if self.kw("ELSE") { Some(Box::new(self.expr(0)?)) } else { None };
        self.expect_kw("END")?;
        Ok(E::Case(Box::new(subject), whens, els))
    }

    fn lod(&mut self) -> Result<E, String> {
        let kind = if self.is_kw("FIXED") || self.is_kw("INCLUDE") || self.is_kw("EXCLUDE") {
            match self.next() {
                Some(Tok::Ident(k)) => k.to_ascii_uppercase(),
                _ => String::new(),
            }
        } else {
            String::new()
        };
        let mut dims = Vec::new();
        if !kind.is_empty() {
            if self.peek() != Some(&Tok::Colon) {
                loop {
                    dims.push(self.expr(0)?);
                    if self.peek() == Some(&Tok::Comma) {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
            }
            self.expect(Tok::Colon)?;
        }
        let body = self.expr(0)?;
        self.expect(Tok::RBrace)?;
        Ok(E::Lod(kind, dims, Box::new(body)))
    }
}

// ------------------------------------------------------------------------------------- analysis

const AGGS: &[&str] = &["SUM", "AVG", "MIN", "MAX", "COUNT", "COUNTD", "MEDIAN", "ATTR", "STDEV", "STDEVP", "VAR", "VARP", "PERCENTILE"];
const TABLE_CALCS: &[&str] = &[
    "RUNNING_SUM",
    "RUNNING_AVG",
    "RUNNING_MIN",
    "RUNNING_MAX",
    "RUNNING_COUNT",
    "WINDOW_SUM",
    "WINDOW_AVG",
    "WINDOW_MIN",
    "WINDOW_MAX",
    "WINDOW_COUNT",
    "WINDOW_MEDIAN",
    "RANK",
    "RANK_DENSE",
    "RANK_UNIQUE",
    "RANK_MODIFIED",
    "INDEX",
    "TOTAL",
    "SIZE",
    "FIRST",
    "LAST",
];

fn is_agg_call(name: &str, args: &[E]) -> bool {
    AGGS.contains(&name) && !((name == "MIN" || name == "MAX") && args.len() == 2)
}

/// Is the expression's result aggregated? (FIXED LODs are row-level; INCLUDE/EXCLUDE aren't.)
fn is_aggregated(e: &E, resolve: &dyn Fn(&str, Option<&str>) -> Option<FieldKind>) -> bool {
    match e {
        E::Field(n, q) => matches!(resolve(n, q.as_deref()), Some(FieldKind::Measure(_))),
        E::Call(n, args) => is_agg_call(n, args) || TABLE_CALCS.contains(&n.as_str()) || args.iter().any(|a| is_aggregated(a, resolve)),
        E::Lod(k, _, _) => k != "FIXED",
        E::Neg(x) | E::Not(x) => is_aggregated(x, resolve),
        E::Bin(_, a, b) => is_aggregated(a, resolve) || is_aggregated(b, resolve),
        E::If(bs, els) => bs.iter().any(|(c, v)| is_aggregated(c, resolve) || is_aggregated(v, resolve)) || els.as_ref().is_some_and(|x| is_aggregated(x, resolve)),
        E::Case(s, ws, els) => is_aggregated(s, resolve) || ws.iter().any(|(a, b)| is_aggregated(a, resolve) || is_aggregated(b, resolve)) || els.as_ref().is_some_and(|x| is_aggregated(x, resolve)),
        _ => false,
    }
}

fn collect_tables(e: &E, resolve: &dyn Fn(&str, Option<&str>) -> Option<FieldKind>, out: &mut Vec<(String, usize)>) {
    let mut add = |t: String| match out.iter_mut().find(|(x, _)| x.eq_ignore_ascii_case(&t)) {
        Some((_, n)) => *n += 1,
        None => out.push((t, 1)),
    };
    match e {
        E::Field(n, q) => {
            if let Some(FieldKind::Column { table, .. }) = resolve(n, q.as_deref()) {
                add(table);
            }
        }
        E::Call(_, args) => args.iter().for_each(|a| collect_tables(a, resolve, out)),
        E::Lod(_, dims, body) => {
            dims.iter().for_each(|a| collect_tables(a, resolve, out));
            collect_tables(body, resolve, out);
        }
        E::Neg(x) | E::Not(x) => collect_tables(x, resolve, out),
        E::Bin(_, a, b) => {
            collect_tables(a, resolve, out);
            collect_tables(b, resolve, out);
        }
        E::If(bs, els) => {
            for (c, v) in bs {
                collect_tables(c, resolve, out);
                collect_tables(v, resolve, out);
            }
            if let Some(x) = els {
                collect_tables(x, resolve, out);
            }
        }
        E::Case(s, ws, els) => {
            collect_tables(s, resolve, out);
            for (a, b) in ws {
                collect_tables(a, resolve, out);
                collect_tables(b, resolve, out);
            }
            if let Some(x) = els {
                collect_tables(x, resolve, out);
            }
        }
        _ => {}
    }
}

// ------------------------------------------------------------------------------------- emitting

/// Where an expression is evaluated.
#[derive(Clone, Debug)]
enum Cx {
    /// Row by row over a table (calculated column, or inside an X-iterator).
    Row(String),
    /// In a filter context (a measure), no row.
    Agg,
}

struct Emitter<'a> {
    resolve: &'a dyn Fn(&str, Option<&str>) -> Option<FieldKind>,
    warnings: Vec<String>,
    order: Option<(String, String)>,
    vars: usize,
    default_table: String,
}

fn col(table: &str, column: &str) -> String {
    format!("{}[{}]", quote_table(table), column.replace(']', "]]"))
}

fn dax_str(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn dax_prec(op: &str) -> u8 {
    match op {
        "||" => 1,
        "&&" => 2,
        "=" | "<>" | "<" | ">" | "<=" | ">=" => 4,
        "&" => 5,
        "+" | "-" => 6,
        "*" | "/" => 7,
        "^" => 9,
        _ => 10,
    }
}

impl Emitter<'_> {
    fn field(&self, n: &str, q: Option<&str>) -> Result<FieldKind, String> {
        (self.resolve)(n, q).ok_or_else(|| match q {
            Some(q) => format!("There's no field [{q}].[{n}]."),
            None => format!("There's no field [{n}] in the model."),
        })
    }

    fn column_of(&self, e: &E) -> Option<(String, String)> {
        match e {
            E::Field(n, q) => match (self.resolve)(n, q.as_deref()) {
                Some(FieldKind::Column { table, column, .. }) => Some((table, column)),
                _ => None,
            },
            _ => None,
        }
    }

    /// The table an iterator over `e` should run on.
    fn row_table(&self, e: &E) -> String {
        let mut counts = Vec::new();
        collect_tables(e, self.resolve, &mut counts);
        counts.iter().max_by_key(|(t, n)| (*n, t.eq_ignore_ascii_case(&self.default_table))).map(|(t, _)| t.clone()).unwrap_or_else(|| self.default_table.clone())
    }

    fn is_text(&self, e: &E) -> bool {
        match e {
            E::Str(_) => true,
            E::Field(n, q) => matches!((self.resolve)(n, q.as_deref()), Some(FieldKind::Column { text: true, .. })),
            E::Call(n, args) => {
                matches!(
                    n.as_str(),
                    "STR" | "LEFT" | "RIGHT" | "MID" | "UPPER" | "LOWER" | "TRIM" | "LTRIM" | "RTRIM" | "REPLACE" | "SPLIT" | "DATENAME" | "SPACE" | "CHAR" | "PROPER"
                ) || (matches!(n.as_str(), "IFNULL" | "IIF" | "ATTR" | "MIN" | "MAX") && args.iter().any(|a| self.is_text(a)))
            }
            E::Bin(op, a, b) if op == "+" => self.is_text(a) || self.is_text(b),
            E::If(bs, els) => bs.iter().any(|(_, v)| self.is_text(v)) || els.as_ref().is_some_and(|x| self.is_text(x)),
            E::Case(_, ws, els) => ws.iter().any(|(_, v)| self.is_text(v)) || els.as_ref().is_some_and(|x| self.is_text(x)),
            _ => false,
        }
    }

    fn args(&mut self, args: &[E], cx: &Cx) -> Result<Vec<String>, String> {
        args.iter().map(|a| self.emit(a, cx)).collect()
    }

    fn emit(&mut self, e: &E, cx: &Cx) -> Result<String, String> {
        Ok(match e {
            E::Num(n) => crate::numfmt::general(*n).replace(',', ""),
            E::Str(s) => dax_str(s),
            E::Bool(b) => if *b { "TRUE()" } else { "FALSE()" }.into(),
            E::Null => "BLANK()".into(),
            E::Date(d) => date_literal(d)?,
            E::Field(n, q) => match self.field(n, q.as_deref())? {
                FieldKind::Measure(m) => format!("[{}]", m.replace(']', "]]")),
                FieldKind::Parameter(p) => {
                    if p.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        format!("@{p}")
                    } else {
                        format!("@[{p}]")
                    }
                }
                FieldKind::Column { table, column, .. } => match cx {
                    Cx::Row(r) if r.eq_ignore_ascii_case(&table) => col(&table, &column),
                    Cx::Row(_) => format!("RELATED({})", col(&table, &column)),
                    Cx::Agg => {
                        return Err(format!(
                            "Cannot mix aggregate and non-aggregate arguments: [{n}] needs an aggregation here, such as SUM([{n}]) or ATTR([{n}])."
                        ));
                    }
                },
            },
            E::Neg(x) => format!("-{}", self.wrap(x, cx, 8)?),
            E::Not(x) => format!("NOT({})", self.emit(x, cx)?),
            E::Bin(op, a, b) => self.binary(op, a, b, cx)?,
            E::If(bs, els) => {
                if let [(c, v)] = bs.as_slice() {
                    let mut s = format!("IF({}, {}", self.emit(c, cx)?, self.emit(v, cx)?);
                    if let Some(x) = els {
                        s += &format!(", {}", self.emit(x, cx)?);
                    }
                    s + ")"
                } else {
                    let mut parts = vec!["TRUE()".to_string()];
                    for (c, v) in bs {
                        parts.push(self.emit(c, cx)?);
                        parts.push(self.emit(v, cx)?);
                    }
                    if let Some(x) = els {
                        parts.push(self.emit(x, cx)?);
                    }
                    format!("SWITCH({})", parts.join(", "))
                }
            }
            E::Case(s, ws, els) => {
                let mut parts = vec![self.emit(s, cx)?];
                for (a, b) in ws {
                    parts.push(self.emit(a, cx)?);
                    parts.push(self.emit(b, cx)?);
                }
                if let Some(x) = els {
                    parts.push(self.emit(x, cx)?);
                }
                format!("SWITCH({})", parts.join(", "))
            }
            E::Lod(kind, dims, body) => self.lod(kind, dims, body, cx, None)?,
            E::Call(name, args) => self.call(name, args, cx)?,
        })
    }

    fn wrap(&mut self, e: &E, cx: &Cx, parent: u8) -> Result<String, String> {
        let s = self.emit(e, cx)?;
        let p = match e {
            E::Bin(op, a, b) => dax_prec(&self.dax_op(op, a, b)),
            _ => 10,
        };
        Ok(if p < parent { format!("({s})") } else { s })
    }

    fn dax_op(&self, op: &str, a: &E, b: &E) -> String {
        match op {
            "+" if self.is_text(a) || self.is_text(b) => "&".into(),
            "==" => "=".into(),
            "!=" => "<>".into(),
            "AND" => "&&".into(),
            "OR" => "||".into(),
            other => other.into(),
        }
    }

    fn binary(&mut self, op: &str, a: &E, b: &E, cx: &Cx) -> Result<String, String> {
        if op == "%" {
            return Ok(format!("MOD({}, {})", self.emit(a, cx)?, self.emit(b, cx)?));
        }
        let d = self.dax_op(op, a, b);
        let p = dax_prec(&d);
        let l = self.wrap(a, cx, p)?;
        let r = self.wrap(b, cx, p + 1)?;
        Ok(format!("{l} {d} {r}"))
    }

    /// An aggregation of `arg`: `plain` over a column, `x` (iterator) over an expression.
    fn agg(&mut self, plain: &str, x: &str, arg: &E, cx: &Cx) -> Result<String, String> {
        if let Cx::Row(_) = cx {
            return Err(format!("{plain} aggregates: it can't be used inside a row-level calculation (only inside a LOD expression)."));
        }
        if let Some((t, c)) = self.column_of(arg) {
            return Ok(format!("{plain}({})", col(&t, &c)));
        }
        if let E::Lod(kind, dims, body) = arg {
            return self.lod(kind, dims, body, cx, Some(x));
        }
        let rt = self.row_table(arg);
        let inner = self.emit(arg, &Cx::Row(rt.clone()))?;
        Ok(format!("{x}({}, {inner})", quote_table(&rt)))
    }

    /// The table an LOD's dimensions are iterated as.
    fn grouping(&mut self, dims: &[(String, String)], body: &E) -> String {
        match dims {
            [(t, c)] => format!("VALUES({})", col(t, c)),
            _ => {
                let base = self.row_table(body);
                format!("SUMMARIZE({}, {})", quote_table(&base), dims.iter().map(|(t, c)| col(t, c)).collect::<Vec<_>>().join(", "))
            }
        }
    }

    fn lod(&mut self, kind: &str, dims: &[E], body: &E, cx: &Cx, outer_x: Option<&str>) -> Result<String, String> {
        let mut dcols = Vec::new();
        for d in dims {
            match self.column_of(d) {
                Some(tc) => dcols.push(tc),
                None => return Err("LOD dimensions must be fields (columns), such as {FIXED [Region] : SUM([Sales])}.".into()),
            }
        }
        let inner = self.emit(body, &Cx::Agg)?;
        if kind.is_empty() || dcols.is_empty() {
            if kind == "INCLUDE" || kind == "EXCLUDE" {
                return Ok(format!("CALCULATE({inner})"));
            }
            return Ok(format!("CALCULATE({inner}, REMOVEFILTERS())"));
        }
        match (kind, cx, outer_x) {
            ("EXCLUDE", _, _) => {
                let rm = dcols.iter().map(|(t, c)| format!("REMOVEFILTERS({})", col(t, c))).collect::<Vec<_>>().join(", ");
                Ok(format!("CALCULATE({inner}, {rm})"))
            }
            ("FIXED", Cx::Row(r), _) => {
                let r = r.clone();
                Ok(self.fixed_with_vars(&inner, &dcols, |t, c| if t.eq_ignore_ascii_case(&r) { col(t, c) } else { format!("RELATED({})", col(t, c)) }))
            }
            ("FIXED", Cx::Agg, Some(x)) => {
                let g = self.grouping(&dcols, body);
                let per = self.fixed_with_vars(&inner, &dcols, col);
                Ok(format!("{x}({g}, {per})"))
            }
            ("FIXED", Cx::Agg, None) => {
                let vals = dcols.iter().map(|(t, c)| format!("VALUES({})", col(t, c))).collect::<Vec<_>>().join(", ");
                Ok(format!("CALCULATE({inner}, REMOVEFILTERS(), {vals})"))
            }
            (_, _, x) => {
                // INCLUDE: per dimension value, then aggregated (SUM unless wrapped in another aggregation).
                if x.is_none() {
                    self.warnings.push("An INCLUDE LOD on its own is summed over its dimensions (wrap it in AVG/MIN/MAX to aggregate differently).".into());
                }
                let g = self.grouping(&dcols, body);
                Ok(format!("{}({g}, CALCULATE({inner}))", x.unwrap_or("SUMX")))
            }
        }
    }

    fn fixed_with_vars(&mut self, inner: &str, dcols: &[(String, String)], rowref: impl Fn(&str, &str) -> String) -> String {
        let mut vars = Vec::new();
        let mut preds = Vec::new();
        for (t, c) in dcols {
            self.vars += 1;
            let v = format!("__k{}", self.vars);
            vars.push(format!("VAR {v} = {}", rowref(t, c)));
            preds.push(format!("{} = {v}", col(t, c)));
        }
        format!("{} RETURN CALCULATE({inner}, REMOVEFILTERS(), {})", vars.join(" "), preds.join(", "))
    }

    fn table_calc(&mut self, name: &str, args: &[E], cx: &Cx) -> Result<String, String> {
        if let Cx::Row(_) = cx {
            return Err(format!("{name} is a table calculation: it works on aggregated values, such as {name}(SUM([Sales]))."));
        }
        let x = match args.first() {
            Some(a) => self.emit(a, cx)?,
            None if name == "INDEX" || name == "SIZE" || name == "FIRST" || name == "LAST" => String::new(),
            None => return Err(format!("{name} needs an aggregated expression.")),
        };
        let Some((ot, oc)) = self.order.clone() else {
            self.warnings.push(format!("{name}: no date column to compute along; computed over the whole selection."));
            return Ok(match name {
                "TOTAL" | "WINDOW_SUM" | "WINDOW_AVG" | "WINDOW_MIN" | "WINDOW_MAX" | "WINDOW_COUNT" | "WINDOW_MEDIAN" => format!("CALCULATE({x}, ALLSELECTED())"),
                _ => x,
            });
        };
        let d = col(&ot, &oc);
        let upto = format!("FILTER(ALLSELECTED({d}), {d} <= MAX({d}))");
        let all = format!("ALLSELECTED({d})");
        self.warnings.push(format!("{name} runs along {d}."));
        Ok(match name {
            "RUNNING_SUM" => format!("SUMX({upto}, CALCULATE({x}))"),
            "RUNNING_AVG" => format!("AVERAGEX({upto}, CALCULATE({x}))"),
            "RUNNING_MIN" => format!("MINX({upto}, CALCULATE({x}))"),
            "RUNNING_MAX" => format!("MAXX({upto}, CALCULATE({x}))"),
            "RUNNING_COUNT" => format!("COUNTX({upto}, CALCULATE({x}))"),
            "WINDOW_SUM" => format!("SUMX({all}, CALCULATE({x}))"),
            "WINDOW_AVG" => format!("AVERAGEX({all}, CALCULATE({x}))"),
            "WINDOW_MIN" => format!("MINX({all}, CALCULATE({x}))"),
            "WINDOW_MAX" => format!("MAXX({all}, CALCULATE({x}))"),
            "WINDOW_COUNT" => format!("COUNTX({all}, CALCULATE({x}))"),
            "WINDOW_MEDIAN" => format!("MEDIANX({all}, CALCULATE({x}))"),
            "RANK" | "RANK_DENSE" | "RANK_UNIQUE" | "RANK_MODIFIED" => format!("RANKX({all}, CALCULATE({x}))"),
            "INDEX" => format!("COUNTROWS({upto})"),
            "SIZE" => format!("COUNTROWS({all})"),
            "FIRST" => format!("1 - COUNTROWS({upto})"),
            "LAST" => format!("COUNTROWS({all}) - COUNTROWS({upto})"),
            _ => format!("CALCULATE({x}, ALLSELECTED())"),
        })
    }

    fn arg(&mut self, args: &[E], i: usize, name: &str, cx: &Cx) -> Result<String, String> {
        match args.get(i) {
            Some(a) => self.emit(a, cx),
            None => Err(format!("{name} needs at least {} argument(s).", i + 1)),
        }
    }

    fn date_part(&self, args: &[E], name: &str) -> Result<String, String> {
        match args.first() {
            Some(E::Str(s)) => Ok(s.to_ascii_lowercase()),
            _ => Err(format!("{name} starts with the date part as text: {name}('month', …).")),
        }
    }

    fn call(&mut self, name: &str, args: &[E], cx: &Cx) -> Result<String, String> {
        let n = args.len();
        if is_agg_call(name, args) {
            let a = args.first().ok_or_else(|| format!("{name} needs a field or expression."))?;
            return match name {
                "SUM" => self.agg("SUM", "SUMX", a, cx),
                "AVG" => self.agg("AVERAGE", "AVERAGEX", a, cx),
                "MIN" => self.agg("MIN", "MINX", a, cx),
                "MAX" => self.agg("MAX", "MAXX", a, cx),
                "MEDIAN" => self.agg("MEDIAN", "MEDIANX", a, cx),
                "COUNT" => self.agg("COUNTA", "COUNTAX", a, cx),
                "STDEV" => self.agg("STDEV.S", "STDEVX.S", a, cx),
                "STDEVP" => self.agg("STDEV.P", "STDEVX.P", a, cx),
                "VAR" => self.agg("VAR.S", "VARX.S", a, cx),
                "VARP" => self.agg("VAR.P", "VARX.P", a, cx),
                "PERCENTILE" => {
                    let k = self.arg(args, 1, name, cx)?;
                    let inner = self.agg("PERCENTILE.INC", "PERCENTILEX.INC", a, cx)?;
                    Ok(format!("{}, {k})", inner.trim_end_matches(')')))
                }
                "COUNTD" => {
                    if let Cx::Row(_) = cx {
                        return Err("COUNTD aggregates: it can't be used in a row-level calculation.".into());
                    }
                    match self.column_of(a) {
                        Some((t, c)) => Ok(format!("DISTINCTCOUNT({})", col(&t, &c))),
                        None => {
                            let rt = self.row_table(a);
                            let inner = self.emit(a, &Cx::Row(rt.clone()))?;
                            Ok(format!("COUNTROWS(DISTINCT(SELECTCOLUMNS({}, \"v\", {inner})))", quote_table(&rt)))
                        }
                    }
                }
                _ => {
                    // ATTR: the value when there's only one, else "*".
                    if let Cx::Row(_) = cx {
                        return Err("ATTR aggregates: it can't be used in a row-level calculation.".into());
                    }
                    match self.column_of(a) {
                        Some((t, c)) => Ok(format!("SELECTEDVALUE({}, \"*\")", col(&t, &c))),
                        None => Err("ATTR takes a field.".into()),
                    }
                }
            };
        }
        if TABLE_CALCS.contains(&name) {
            return self.table_calc(name, args, cx);
        }
        let a = |me: &mut Self, i: usize| me.arg(args, i, name, cx);
        Ok(match name {
            "IIF" => format!("IF({}, {}, {})", a(self, 0)?, a(self, 1)?, a(self, 2)?),
            "ZN" => format!("COALESCE({}, 0)", a(self, 0)?),
            "IFNULL" => format!("COALESCE({}, {})", a(self, 0)?, a(self, 1)?),
            "ISNULL" => format!("ISBLANK({})", a(self, 0)?),
            "MIN" | "MAX" => format!("{name}({}, {})", a(self, 0)?, a(self, 1)?),
            // Strings.
            "LEN" | "UPPER" | "LOWER" | "TRIM" | "ABS" | "SQRT" | "EXP" | "LN" | "SIGN" | "YEAR" | "MONTH" | "DAY" | "QUARTER" | "SIN" | "COS" | "TAN" | "ASIN" | "ACOS" | "ATAN" | "DEGREES" | "RADIANS" => {
                format!("{name}({})", a(self, 0)?)
            }
            "LTRIM" | "RTRIM" => format!("TRIM({})", a(self, 0)?),
            "LEFT" | "RIGHT" => format!("{name}({}, {})", a(self, 0)?, a(self, 1)?),
            "MID" => {
                let s = a(self, 0)?;
                let len = if n > 2 { a(self, 2)? } else { format!("LEN({s})") };
                format!("MID({s}, {}, {len})", a(self, 1)?)
            }
            "CONTAINS" => format!("CONTAINSSTRINGEXACT({}, {})", a(self, 0)?, a(self, 1)?),
            "STARTSWITH" => {
                let (s, p) = (a(self, 0)?, a(self, 1)?);
                format!("LEFT({s}, LEN({p})) = {p}")
            }
            "ENDSWITH" => {
                let (s, p) = (a(self, 0)?, a(self, 1)?);
                format!("RIGHT({s}, LEN({p})) = {p}")
            }
            "REPLACE" => format!("SUBSTITUTE({}, {}, {})", a(self, 0)?, a(self, 1)?, a(self, 2)?),
            "FIND" | "FINDNTH" => format!("FIND({}, {}, 1, 0)", a(self, 1)?, a(self, 0)?),
            "SPLIT" => format!("PATHITEM(SUBSTITUTE({}, {}, \"|\"), {})", a(self, 0)?, a(self, 1)?, a(self, 2)?),
            "STR" => format!("FORMAT({}, \"General\")", a(self, 0)?),
            "INT" => format!("TRUNC({})", a(self, 0)?),
            "FLOAT" => format!("VALUE({})", a(self, 0)?),
            "ASCII" => format!("UNICODE({})", a(self, 0)?),
            "CHAR" => format!("UNICHAR({})", a(self, 0)?),
            "SPACE" => format!("REPT(\" \", {})", a(self, 0)?),
            "PROPER" => format!("UPPER(LEFT({0}, 1)) & LOWER(MID({0}, 2, LEN({0})))", a(self, 0)?),
            // Numbers.
            "ROUND" => format!("ROUND({}, {})", a(self, 0)?, if n > 1 { a(self, 1)? } else { "0".into() }),
            "CEILING" => format!("ROUNDUP({}, 0)", a(self, 0)?),
            "FLOOR" => format!("ROUNDDOWN({}, 0)", a(self, 0)?),
            "POWER" => format!("POWER({}, {})", a(self, 0)?, a(self, 1)?),
            "LOG" => format!("LOG({}, {})", a(self, 0)?, if n > 1 { a(self, 1)? } else { "10".into() }),
            "DIV" => format!("QUOTIENT({}, {})", a(self, 0)?, a(self, 1)?),
            "PI" => "PI()".into(),
            "SQUARE" => format!("POWER({}, 2)", a(self, 0)?),
            // Dates.
            "TODAY" => "TODAY()".into(),
            "NOW" => "NOW()".into(),
            "MAKEDATE" => format!("DATE({}, {}, {})", a(self, 0)?, a(self, 1)?, a(self, 2)?),
            "DATE" => format!("DATEVALUE({})", a(self, 0)?),
            "DATEPART" => {
                let part = self.date_part(args, name)?;
                let d = a(self, 1)?;
                match part.as_str() {
                    "year" => format!("YEAR({d})"),
                    "quarter" => format!("QUARTER({d})"),
                    "month" => format!("MONTH({d})"),
                    "day" => format!("DAY({d})"),
                    "week" => format!("WEEKNUM({d})"),
                    "weekday" => format!("WEEKDAY({d})"),
                    "dayofyear" => format!("({d} - DATE(YEAR({d}), 1, 1) + 1)"),
                    "hour" => format!("HOUR({d})"),
                    "minute" => format!("MINUTE({d})"),
                    "second" => format!("SECOND({d})"),
                    other => return Err(format!("DATEPART: unknown date part '{other}'.")),
                }
            }
            "DATENAME" => {
                let part = self.date_part(args, name)?;
                let d = a(self, 1)?;
                let f = match part.as_str() {
                    "year" => "yyyy",
                    "quarter" => "\"Q\"q",
                    "month" => "mmmm",
                    "day" => "d",
                    "weekday" => "dddd",
                    other => return Err(format!("DATENAME: unknown date part '{other}'.")),
                };
                format!("FORMAT({d}, {})", dax_str(f))
            }
            "DATETRUNC" => {
                let part = self.date_part(args, name)?;
                let d = a(self, 1)?;
                match part.as_str() {
                    "year" => format!("DATE(YEAR({d}), 1, 1)"),
                    "quarter" => format!("DATE(YEAR({d}), (QUARTER({d}) - 1) * 3 + 1, 1)"),
                    "month" => format!("DATE(YEAR({d}), MONTH({d}), 1)"),
                    "week" => format!("({d} - WEEKDAY({d}) + 1)"),
                    "day" => format!("DATE(YEAR({d}), MONTH({d}), DAY({d}))"),
                    other => return Err(format!("DATETRUNC: unknown date part '{other}'.")),
                }
            }
            "DATEADD" => {
                let part = self.date_part(args, name)?;
                let (k, d) = (a(self, 1)?, a(self, 2)?);
                match part.as_str() {
                    "year" => format!("EDATE({d}, ({k}) * 12)"),
                    "quarter" => format!("EDATE({d}, ({k}) * 3)"),
                    "month" => format!("EDATE({d}, {k})"),
                    "week" => format!("({d} + ({k}) * 7)"),
                    "day" => format!("({d} + {k})"),
                    "hour" => format!("({d} + ({k}) / 24)"),
                    "minute" => format!("({d} + ({k}) / 1440)"),
                    other => return Err(format!("DATEADD: unknown date part '{other}'.")),
                }
            }
            "DATEDIFF" => {
                let part = self.date_part(args, name)?;
                let unit = match part.as_str() {
                    "year" => "YEAR",
                    "quarter" => "QUARTER",
                    "month" => "MONTH",
                    "week" => "WEEK",
                    "day" => "DAY",
                    "hour" => "HOUR",
                    "minute" => "MINUTE",
                    "second" => "SECOND",
                    other => return Err(format!("DATEDIFF: unknown date part '{other}'.")),
                };
                format!("DATEDIFF({}, {}, {unit})", a(self, 1)?, a(self, 2)?)
            }
            _ => {
                // Functions with the same name and arguments in DAX.
                let known = vizcraft_datamodel::model::Model::new().evaluate(&format!("{name}()")).err().is_none_or(|e| !e.to_string().contains("unknown function"));
                if !known {
                    return Err(format!("`{name}` isn't a function VizCraft can translate."));
                }
                format!("{name}({})", self.args(args, cx)?.join(", "))
            }
        })
    }
}

fn date_literal(d: &str) -> Result<String, String> {
    let s = d.trim();
    let date = s.get(..10).unwrap_or(s);
    let parts: Vec<&str> = date.split(['-', '/']).collect();
    let nums: Vec<i64> = parts.iter().filter_map(|p| p.trim().parse().ok()).collect();
    match nums.as_slice() {
        [y, m, dd] if *y > 31 => Ok(format!("DATE({y}, {m}, {dd})")),
        [m, dd, y] if *y > 31 => Ok(format!("DATE({y}, {m}, {dd})")),
        _ => Err(format!("#{d}# isn't a date (use #yyyy-mm-dd#).")),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn res(name: &str, q: Option<&str>) -> Option<FieldKind> {
        if q == Some("Parameters") {
            return Some(FieldKind::Parameter(name.into()));
        }
        let (t, text) = match name {
            "Sales" | "Quantity" | "Discount" | "Order Date" | "Customer" | "OrderID" => ("Orders", name == "Customer"),
            "Category" | "Product" => ("Product", true),
            "Region" => ("Store", true),
            "Profit Ratio" => return Some(FieldKind::Measure(name.into())),
            "Growth" => return Some(FieldKind::Parameter(name.into())),
            _ => return None,
        };
        Some(FieldKind::Column { table: t.into(), column: name.into(), text })
    }

    fn tr(s: &str) -> Translation {
        translate(s, "Orders", &res, Some(("Orders".into(), "Order Date".into()))).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn row_level() {
        let t = tr("IF [Sales] > 100 THEN 'Big' ELSEIF [Sales] > 10 THEN 'Mid' ELSE 'Small' END");
        assert!(!t.aggregated);
        assert_eq!(t.dax, "SWITCH(TRUE(), 'Orders'[Sales] > 100, \"Big\", 'Orders'[Sales] > 10, \"Mid\", \"Small\")".replace("'Orders'", "Orders"));
        assert_eq!(tr("IIF([Discount] > 0, 'Yes', 'No')").dax, "IF(Orders[Discount] > 0, \"Yes\", \"No\")");
        assert_eq!(tr("ZN([Discount]) * 2").dax, "COALESCE(Orders[Discount], 0) * 2");
        assert_eq!(tr("[Customer] + ' / ' + [Region]").dax, "Orders[Customer] & \" / \" & RELATED(Store[Region])");
        assert_eq!(tr("CASE [Region] WHEN 'East' THEN 1 WHEN 'West' THEN 2 ELSE 0 END").home_table, "Store");
        assert_eq!(tr("DATETRUNC('month', [Order Date])").dax, "DATE(YEAR(Orders[Order Date]), MONTH(Orders[Order Date]), 1)");
        assert_eq!(tr("DATEPART('year', [Order Date])").dax, "YEAR(Orders[Order Date])");
        assert_eq!(tr("DATEADD('quarter', 1, [Order Date])").dax, "EDATE(Orders[Order Date], (1) * 3)");
        assert_eq!(tr("DATEDIFF('day', [Order Date], TODAY())").dax, "DATEDIFF(Orders[Order Date], TODAY(), DAY)");
        assert_eq!(tr("CONTAINS([Customer], 'Inc') AND NOT ISNULL([Discount])").dax, "CONTAINSSTRINGEXACT(Orders[Customer], \"Inc\") && NOT(ISBLANK(Orders[Discount]))");
        assert_eq!(tr("STARTSWITH(UPPER([Customer]), 'A')").dax, "LEFT(UPPER(Orders[Customer]), LEN(\"A\")) = \"A\"");
        assert_eq!(tr("MID([Customer], 2)").dax, "MID(Orders[Customer], 2, LEN(Orders[Customer]))");
        assert_eq!(tr("[Sales] * (1 + [Parameters].[Growth])").dax, "Orders[Sales] * (1 + @Growth)");
        assert_eq!(tr("[Sales] % 7").dax, "MOD(Orders[Sales], 7)");
        assert_eq!(tr("#2024-03-01# + 1").dax, "DATE(2024, 3, 1) + 1");
        assert_eq!(tr("SPLIT([Customer], ' ', 1)").dax, "PATHITEM(SUBSTITUTE(Orders[Customer], \" \", \"|\"), 1)");
    }

    #[test]
    fn aggregates() {
        let t = tr("SUM([Sales]) / SUM([Quantity])");
        assert!(t.aggregated);
        assert_eq!(t.dax, "SUM(Orders[Sales]) / SUM(Orders[Quantity])");
        assert_eq!(tr("AVG([Sales] * [Discount])").dax, "AVERAGEX(Orders, Orders[Sales] * Orders[Discount])");
        assert_eq!(tr("COUNTD([Customer])").dax, "DISTINCTCOUNT(Orders[Customer])");
        assert_eq!(tr("ATTR([Region])").dax, "SELECTEDVALUE(Store[Region], \"*\")");
        assert_eq!(tr("[Profit Ratio] * 100").dax, "[Profit Ratio] * 100");
        assert_eq!(tr("IF SUM([Sales]) > 1000 THEN 'High' ELSE 'Low' END").dax, "IF(SUM(Orders[Sales]) > 1000, \"High\", \"Low\")");
        assert!(translate("SUM([Sales]) + [Quantity]", "Orders", &res, None).unwrap_err().contains("Cannot mix"));
        assert!(translate("[Nope] + 1", "Orders", &res, None).unwrap_err().contains("Nope"));
    }

    #[test]
    fn lods() {
        let t = tr("{FIXED [Customer] : MIN([Order Date])}");
        assert!(!t.aggregated);
        assert_eq!(t.dax, "VAR __k1 = Orders[Customer] RETURN CALCULATE(MIN(Orders[Order Date]), REMOVEFILTERS(), Orders[Customer] = __k1)");
        let t = tr("AVG({FIXED [Region] : SUM([Sales])})");
        assert!(t.aggregated);
        assert_eq!(t.dax, "AVERAGEX(VALUES(Store[Region]), VAR __k1 = Store[Region] RETURN CALCULATE(SUM(Orders[Sales]), REMOVEFILTERS(), Store[Region] = __k1))");
        assert_eq!(tr("{EXCLUDE [Region] : SUM([Sales])}").dax, "CALCULATE(SUM(Orders[Sales]), REMOVEFILTERS(Store[Region]))");
        assert_eq!(tr("AVG({INCLUDE [Customer] : SUM([Sales])})").dax, "AVERAGEX(VALUES(Orders[Customer]), CALCULATE(SUM(Orders[Sales])))");
        assert_eq!(tr("SUM([Sales]) / {SUM([Sales])}").dax, "SUM(Orders[Sales]) / CALCULATE(SUM(Orders[Sales]), REMOVEFILTERS())");
    }

    #[test]
    fn table_calcs_and_passthrough() {
        assert_eq!(tr("RUNNING_SUM(SUM([Sales]))").dax, "SUMX(FILTER(ALLSELECTED(Orders[Order Date]), Orders[Order Date] <= MAX(Orders[Order Date])), CALCULATE(SUM(Orders[Sales])))");
        assert_eq!(tr("WINDOW_AVG(SUM([Sales]))").dax, "AVERAGEX(ALLSELECTED(Orders[Order Date]), CALCULATE(SUM(Orders[Sales])))");
        assert_eq!(tr("RANK(SUM([Sales]))").dax, "RANKX(ALLSELECTED(Orders[Order Date]), CALCULATE(SUM(Orders[Sales])))");
        assert_eq!(to_dax("SUM(Sales[Amount])", "Sales").unwrap(), "SUM(Sales[Amount])");
        assert_eq!(to_dax("CALCULATE(SUM(Sales[Amount]), ALL(Sales))", "Sales").unwrap(), "CALCULATE(SUM(Sales[Amount]), ALL(Sales))");
        assert_eq!(to_dax("[Amount] * 2", "Sales").unwrap(), "Sales[Amount] * 2");
    }

    #[test]
    fn translations_run_on_the_sample() {
        let p = crate::sample::sales();
        let b = crate::build::build(&p, &vizcraft_query::NoHost, &Default::default());
        for (calc, agg) in [
            ("SUM([Quantity] * [Unit Price])", true),
            ("IF [Quantity] >= 3 THEN 'Bulk' ELSE 'Single' END", false),
            ("{FIXED [Region] : SUM([Quantity])}", false),
            ("AVG({FIXED [Category] : SUM([Quantity])})", true),
            ("ZN([Discount]) + DATEPART('month', [Date])", false),
            ("RUNNING_SUM(SUM([Quantity]))", true),
            ("[Total Sales] / {FIXED : SUM([Quantity])}", true),
        ] {
            let t = translate_in(&p, &b, calc, "Sales").unwrap_or_else(|e| panic!("{calc}: {e}"));
            assert_eq!(t.aggregated, agg, "{calc}");
            if agg {
                b.model.evaluate(&t.dax).unwrap_or_else(|e| panic!("{calc} → {}: {e}", t.dax));
            } else {
                let mut m = (*b.model).clone();
                m.add_calculated_column(&t.home_table, "Test", &t.dax).unwrap_or_else(|e| panic!("{calc} → {}: {e}", t.dax));
            }
        }
    }
}
