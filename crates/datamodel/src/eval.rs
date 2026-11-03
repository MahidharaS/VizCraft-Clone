//! The DAX evaluator: expressions run against the columnar store under a filter context and a
//! row context, with CALCULATE's context transition and filter arguments.

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use crate::dax::{BinOp, Expr, UnOp};
use crate::filter::{Ctx, Filter};
use crate::model::Model;
use crate::store::RowSet;
use crate::value::{Key, Value, compare, sort_order};
use crate::{Error, Result};

mod ext;

/// How deep measures may call measures.
pub const MAX_MEASURE_DEPTH: usize = 64;
/// Evaluation steps one call may take before it is stopped (bounds runaway iterations).
const MAX_STEPS: u64 = 500_000_000;
/// The most rows a computed table may have.
pub const MAX_TABLE_ROWS: usize = 5_000_000;

/// A column of a computed table: a model column, or one a function named.
#[derive(Clone, Debug, PartialEq)]
pub enum ColName {
    Model(usize, usize),
    Named(String),
}

impl ColName {
    pub fn display(&self, m: &Model) -> String {
        match self {
            ColName::Model(t, c) => m.column_name(*t, *c),
            ColName::Named(n) => n.clone(),
        }
    }

    fn has_name(&self, m: &Model, name: &str) -> bool {
        match self {
            ColName::Model(t, c) => m.tables.get(*t).and_then(|t| t.column(*c)).is_some_and(|c| c.name.eq_ignore_ascii_case(name)),
            ColName::Named(n) => n.eq_ignore_ascii_case(name),
        }
    }
}

/// A table value: rows of a model table, or computed rows.
#[derive(Clone, Debug)]
pub enum TableVal {
    Rows { table: usize, rows: Vec<u32> },
    Values { columns: Vec<ColName>, data: Vec<Vec<Value>> },
}

impl TableVal {
    pub fn len(&self) -> usize {
        match self {
            TableVal::Rows { rows, .. } => rows.len(),
            TableVal::Values { data, .. } => data.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The table keeping only the rows whose index is in `keep`, in that order.
    fn select(&self, keep: &[usize]) -> TableVal {
        match self {
            TableVal::Rows { table, rows } => TableVal::Rows { table: *table, rows: keep.iter().filter_map(|i| rows.get(*i).copied()).collect() },
            TableVal::Values { columns, data } => {
                TableVal::Values { columns: columns.clone(), data: keep.iter().filter_map(|i| data.get(*i).cloned()).collect() }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum Val {
    Scalar(Value),
    Table(Rc<TableVal>),
}

/// One level of row context.
#[derive(Clone, Debug)]
pub enum Frame {
    Row { table: usize, row: u32 },
    Tuple { columns: Rc<Vec<ColName>>, values: Rc<Vec<Value>> },
}

/// The row context of row `i` of a table.
pub fn frame_of(tv: &TableVal, i: usize) -> Option<Frame> {
    match tv {
        TableVal::Rows { table, rows } => rows.get(i).map(|r| Frame::Row { table: *table, row: *r }),
        TableVal::Values { columns, data } => data.get(i).map(|v| Frame::Tuple { columns: Rc::new(columns.clone()), values: Rc::new(v.clone()) }),
    }
}

fn frames(tv: &TableVal) -> Vec<Frame> {
    match tv {
        TableVal::Rows { table, rows } => rows.iter().map(|r| Frame::Row { table: *table, row: *r }).collect(),
        TableVal::Values { columns, data } => {
            let cols = Rc::new(columns.clone());
            data.iter().map(|v| Frame::Tuple { columns: cols.clone(), values: Rc::new(v.clone()) }).collect()
        }
    }
}

/// Variables and row context.
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub vars: Vec<(String, Val)>,
    pub frames: Vec<Frame>,
    pub depth: usize,
}

impl Env {
    pub fn row(table: usize, row: u32) -> Env {
        Env { frames: vec![Frame::Row { table, row }], ..Env::default() }
    }

    fn with_frame(&self, f: Frame) -> Env {
        let mut e = self.clone();
        e.frames.push(f);
        e
    }

    /// The environment inside CALCULATE: row context is gone (turned into filters), variables stay.
    fn without_rows(&self) -> Env {
        Env { vars: self.vars.clone(), frames: Vec::new(), depth: self.depth }
    }
}

fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Eval(msg.into()))
}

fn arg<'a>(args: &'a [Expr], i: usize, f: &str) -> Result<&'a Expr> {
    args.get(i).ok_or_else(|| Error::Eval(format!("{f} needs at least {} argument(s)", i + 1)))
}

const TIME_TABLE_FNS: &[&str] = &[
    "DATESYTD",
    "DATESQTD",
    "DATESMTD",
    "SAMEPERIODLASTYEAR",
    "DATEADD",
    "PREVIOUSYEAR",
    "PREVIOUSQUARTER",
    "PREVIOUSMONTH",
    "DATESBETWEEN",
    "DATESINPERIOD",
    "PARALLELPERIOD",
    "NEXTDAY",
    "NEXTMONTH",
    "NEXTQUARTER",
    "NEXTYEAR",
    "PREVIOUSDAY",
    "FIRSTDATE",
    "LASTDATE",
    "STARTOFMONTH",
    "STARTOFQUARTER",
    "STARTOFYEAR",
    "ENDOFMONTH",
    "ENDOFQUARTER",
    "ENDOFYEAR",
];

pub struct Evaluator<'m> {
    pub m: &'m Model,
    /// Query-scoped measures (`DEFINE MEASURE`).
    local: &'m [(String, Expr)],
    steps: Cell<u64>,
}

impl<'m> Evaluator<'m> {
    pub fn new(m: &'m Model, local: &'m [(String, Expr)]) -> Evaluator<'m> {
        Evaluator { m, local, steps: Cell::new(0) }
    }

    fn tick(&self) -> Result<()> {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        if n > MAX_STEPS { err("the calculation is too large to finish") } else { Ok(()) }
    }

    pub fn scalar(&self, e: &Expr, ctx: &Ctx, env: &Env) -> Result<Value> {
        match self.eval(e, ctx, env)? {
            Val::Scalar(v) => Ok(v),
            Val::Table(t) => match &*t {
                _ if t.is_empty() => Ok(Value::Blank),
                TableVal::Values { columns, data } if columns.len() == 1 && data.len() == 1 => {
                    Ok(data.first().and_then(|r| r.first()).cloned().unwrap_or_default())
                }
                _ => err("a table of multiple values was supplied where a single value was expected"),
            },
        }
    }

    pub fn table(&self, e: &Expr, ctx: &Ctx, env: &Env) -> Result<Rc<TableVal>> {
        match self.eval(e, ctx, env)? {
            Val::Table(t) => Ok(t),
            Val::Scalar(_) => err("a table expression was expected"),
        }
    }

    fn eval(&self, e: &Expr, ctx: &Ctx, env: &Env) -> Result<Val> {
        self.tick()?;
        let s = Val::Scalar;
        Ok(match e {
            Expr::Number(n) => s(Value::Number(*n)),
            Expr::Text(t) => s(Value::text(t)),
            Expr::Bool(b) => s(Value::Bool(*b)),
            Expr::Column { table, column } => {
                let (t, c) = self.m.column_ref(table, column)?;
                s(self.row_value(t, c, env)?)
            }
            Expr::Bracket(name) => self.bracket(name, ctx, env)?,
            Expr::Name(n) => self.name(n, ctx, env)?,
            Expr::Unary(UnOp::Neg, x) => s(neg(&self.scalar(x, ctx, env)?)?),
            Expr::Unary(UnOp::Not, x) => s(Value::Bool(!self.scalar(x, ctx, env)?.truthy()?)),
            Expr::Binary(op, a, b) => s(self.binary(*op, a, b, ctx, env)?),
            Expr::Call(name, args) => self.call(name, args, ctx, env)?,
            Expr::Var(bindings, body) => {
                let mut env2 = env.clone();
                for (n, x) in bindings {
                    let v = self.eval(x, ctx, &env2)?;
                    env2.vars.push((n.clone(), v));
                }
                self.eval(body, ctx, &env2)?
            }
            Expr::TableCtor(rows) => Val::Table(Rc::new(self.table_ctor(rows, ctx, env)?)),
        })
    }

    fn table_ctor(&self, rows: &[Vec<Expr>], ctx: &Ctx, env: &Env) -> Result<TableVal> {
        let width = rows.iter().map(Vec::len).max().unwrap_or(1);
        let columns = (1..=width).map(|i| ColName::Named(if width == 1 { "Value".into() } else { format!("Value{i}") })).collect();
        let mut data = Vec::with_capacity(rows.len());
        for r in rows {
            let mut out = Vec::with_capacity(width);
            for x in r {
                out.push(self.scalar(x, ctx, env)?);
            }
            out.resize(width, Value::Blank);
            data.push(out);
        }
        Ok(TableVal::Values { columns, data })
    }

    /// The value of a column in the current row context.
    fn row_value(&self, t: usize, c: usize, env: &Env) -> Result<Value> {
        for f in env.frames.iter().rev() {
            match f {
                Frame::Row { table, row } if *table == t => {
                    return Ok(self.m.tables.get(t).and_then(|tb| tb.column(c)).map(|col| col.value(*row as usize).clone()).unwrap_or_default());
                }
                Frame::Tuple { columns, values } => {
                    if let Some(i) = columns.iter().position(|cn| *cn == ColName::Model(t, c)) {
                        return Ok(values.get(i).cloned().unwrap_or_default());
                    }
                }
                Frame::Row { .. } => {}
            }
        }
        err(format!("a single value for column {} cannot be determined (there is no row context for it)", self.m.column_name(t, c)))
    }

    fn bracket(&self, name: &str, ctx: &Ctx, env: &Env) -> Result<Val> {
        if let Some((_, e)) = self.local.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            return self.measure(e, ctx, env);
        }
        if let Some(m) = self.m.measure(name) {
            return self.measure(&m.expr, ctx, env);
        }
        for f in env.frames.iter().rev() {
            match f {
                Frame::Row { table, row } => {
                    if let Some(col) = self.m.tables.get(*table).and_then(|t| t.column_index(name).and_then(|c| t.column(c))) {
                        return Ok(Val::Scalar(col.value(*row as usize).clone()));
                    }
                }
                Frame::Tuple { columns, values } => {
                    if let Some(i) = columns.iter().position(|cn| cn.has_name(self.m, name)) {
                        return Ok(Val::Scalar(values.get(i).cloned().unwrap_or_default()));
                    }
                }
            }
        }
        err(format!("[{name}] is not a measure, nor a column of the current row"))
    }

    /// Evaluates a measure: the row context becomes filters (context transition).
    fn measure(&self, e: &Expr, ctx: &Ctx, env: &Env) -> Result<Val> {
        if env.depth >= MAX_MEASURE_DEPTH {
            return err("measures refer to each other in a loop (or nest too deeply)");
        }
        let inner = Env { vars: Vec::new(), frames: Vec::new(), depth: env.depth + 1 };
        if env.frames.is_empty() {
            return self.eval(e, ctx, &inner);
        }
        let ctx2 = self.transition(ctx, env);
        self.eval(e, &ctx2, &inner)
    }

    /// The filter context with the row context added as filters.
    fn transition(&self, ctx: &Ctx, env: &Env) -> Ctx {
        let mut c = ctx.clone();
        for f in &env.frames {
            match f {
                Frame::Row { table, row } => {
                    let n = self.m.tables.get(*table).map(|t| t.row_count()).unwrap_or(0);
                    c.replace(Filter::Rows { table: *table, rows: Rc::new(RowSet::from_rows(n, [*row])) });
                }
                Frame::Tuple { columns, values } => {
                    for (cn, v) in columns.iter().zip(values.iter()) {
                        if let ColName::Model(t, col) = cn {
                            c.replace(Filter::Values { table: *t, column: *col, keys: Rc::new(HashSet::from([v.key()])) });
                        }
                    }
                }
            }
        }
        c
    }

    fn name(&self, n: &str, ctx: &Ctx, env: &Env) -> Result<Val> {
        if let Some((_, v)) = env.vars.iter().rev().find(|(name, _)| name.eq_ignore_ascii_case(n)) {
            return Ok(v.clone());
        }
        let t = self.m.table_index(n).ok_or_else(|| Error::Eval(format!("`{n}` is not a table or a variable")))?;
        Ok(Val::Table(Rc::new(self.visible_rows(t, ctx))))
    }

    fn visible_rows(&self, t: usize, ctx: &Ctx) -> TableVal {
        TableVal::Rows { table: t, rows: ctx.visible(self.m, t).iter().map(|r| r as u32).collect() }
    }

    fn binary(&self, op: BinOp, a: &Expr, b: &Expr, ctx: &Ctx, env: &Env) -> Result<Value> {
        match op {
            BinOp::And => Ok(Value::Bool(self.scalar(a, ctx, env)?.truthy()? && self.scalar(b, ctx, env)?.truthy()?)),
            BinOp::Or => Ok(Value::Bool(self.scalar(a, ctx, env)?.truthy()? || self.scalar(b, ctx, env)?.truthy()?)),
            BinOp::In => {
                let x = self.scalar(a, ctx, env)?;
                let t = self.table(b, ctx, env)?;
                let first = |i: usize| -> Value {
                    match &*t {
                        TableVal::Values { data, .. } => data.get(i).and_then(|r| r.first()).cloned().unwrap_or_default(),
                        TableVal::Rows { table, rows } => rows
                            .get(i)
                            .and_then(|r| self.m.tables.get(*table).and_then(|tb| tb.column(0)).map(|c| c.value(*r as usize).clone()))
                            .unwrap_or_default(),
                    }
                };
                Ok(Value::Bool((0..t.len()).any(|i| compare(&x, &first(i)) == std::cmp::Ordering::Equal)))
            }
            _ => binary_scalar(op, &self.scalar(a, ctx, env)?, &self.scalar(b, ctx, env)?),
        }
    }

    // ------------------------------------------------------------ column arguments

    /// `(table, column)` of a column argument: `T[C]`, or `[C]` of the current row's table.
    fn col_arg(&self, e: &Expr, env: &Env, f: &str) -> Result<(usize, usize)> {
        match e {
            Expr::Column { table, column } => self.m.column_ref(table, column),
            Expr::Bracket(name) => {
                for fr in env.frames.iter().rev() {
                    if let Frame::Row { table, .. } = fr
                        && let Some(c) = self.m.tables.get(*table).and_then(|t| t.column_index(name))
                    {
                        return Ok((*table, c));
                    }
                }
                err(format!("{f} needs a column reference such as Table[Column]"))
            }
            _ => err(format!("{f} needs a column reference such as Table[Column]")),
        }
    }

    fn column_values<'a>(&'a self, t: usize, c: usize, rows: &'a RowSet) -> impl Iterator<Item = &'a Value> + 'a {
        let col = self.m.tables.get(t).and_then(|tb| tb.column(c));
        rows.iter().filter_map(move |r| col.map(|c| c.value(r)))
    }

    /// Distinct value tuples of `cols` of table `t` over `rows`, sorted.
    fn distinct_tuples(&self, t: usize, cols: &[usize], rows: impl Iterator<Item = usize>) -> Result<Vec<Vec<Value>>> {
        let Some(tb) = self.m.tables.get(t) else { return Ok(vec![]) };
        let mut seen: HashSet<Vec<Key>> = HashSet::new();
        let mut out = Vec::new();
        for r in rows {
            let tuple: Vec<Value> = cols.iter().map(|c| tb.column(*c).map(|col| col.value(r).clone()).unwrap_or_default()).collect();
            if seen.insert(tuple.iter().map(Value::key).collect()) {
                out.push(tuple);
                if out.len() > MAX_TABLE_ROWS {
                    return err("the result has too many rows");
                }
            }
        }
        out.sort_by(|a, b| a.iter().zip(b.iter()).map(|(x, y)| sort_order(x, y)).find(|o| o.is_ne()).unwrap_or(std::cmp::Ordering::Equal));
        Ok(out)
    }

    // ------------------------------------------------------------ functions

    fn call(&self, name: &str, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<Val> {
        let s = |v: Result<Value>| v.map(Val::Scalar);
        let t = |v: Result<TableVal>| v.map(|t| Val::Table(Rc::new(t)));
        if let Some(r) = self.call_ext(name, args, ctx, env) {
            return r;
        }
        match name {
            "MIN" | "MAX" if args.len() == 2 => s(self.pure(name, args, ctx, env)),
            "SUM"
            | "AVERAGE"
            | "MIN"
            | "MAX"
            | "COUNT"
            | "COUNTA"
            | "COUNTBLANK"
            | "DISTINCTCOUNT"
            | "DISTINCTCOUNTNOBLANK"
            | "PRODUCT"
            | "MEDIAN" => s(self.aggregate(name, args, ctx, env)),
            "SUMX" | "AVERAGEX" | "MINX" | "MAXX" | "COUNTX" | "COUNTAX" | "PRODUCTX" | "MEDIANX" | "CONCATENATEX" | "RANKX" => {
                s(self.iterate(name, args, ctx, env))
            }
            "COUNTROWS" => s(self.table(arg(args, 0, name)?, ctx, env).map(|t| Value::Number(t.len() as f64))),
            "CALCULATE" => {
                let ctx2 = self.filter_args(args.get(1..).unwrap_or_default(), ctx, env, true)?;
                self.eval(arg(args, 0, name)?, &ctx2, &env.without_rows())
            }
            "CALCULATETABLE" => {
                let ctx2 = self.filter_args(args.get(1..).unwrap_or_default(), ctx, env, true)?;
                self.eval(arg(args, 0, name)?, &ctx2, &env.without_rows())
            }
            "FILTER" => t(self.filter_fn(args, ctx, env)),
            "ALL" | "REMOVEFILTERS" | "ALLNOBLANKROW" => t(self.all_fn(args, env)),
            "VALUES" | "DISTINCT" => t(self.values_fn(name, args, ctx, env)),
            "RELATEDTABLE" => {
                let tn = match arg(args, 0, name)? {
                    Expr::Name(n) => self.m.table_index(n).ok_or_else(|| Error::Eval(format!("no table named `{n}`")))?,
                    _ => return err("RELATEDTABLE needs a table name"),
                };
                Ok(Val::Table(Rc::new(self.visible_rows(tn, &self.transition(ctx, env)))))
            }
            "SUMMARIZECOLUMNS" => t(self.summarize_columns(args, ctx, env)),
            "SUMMARIZE" => t(self.summarize(args, ctx, env)),
            "ADDCOLUMNS" | "SELECTCOLUMNS" => t(self.add_columns(name, args, ctx, env)),
            "TOPN" => t(self.topn(args, ctx, env)),
            "GENERATESERIES" => t(self.series(args, ctx, env)),
            _ if TIME_TABLE_FNS.contains(&name) => t(self.time_table(name, args, ctx, env)),
            "TOTALYTD" | "TOTALQTD" | "TOTALMTD" => {
                let dates = arg(args, 1, name)?.clone();
                let period = match name {
                    "TOTALYTD" => "DATESYTD",
                    "TOTALQTD" => "DATESQTD",
                    _ => "DATESMTD",
                };
                let mut cargs = vec![arg(args, 0, name)?.clone(), Expr::Call(period.into(), vec![dates])];
                cargs.extend(args.get(2..).unwrap_or_default().iter().cloned());
                self.call("CALCULATE", &cargs, ctx, env)
            }
            "RELATED" => s(self.related(args, env)),
            "LOOKUPVALUE" => s(self.lookup_value(args, ctx, env)),
            "HASONEVALUE" | "SELECTEDVALUE" | "ISFILTERED" | "ISCROSSFILTERED" => s(self.filter_info(name, args, ctx, env)),
            _ => s(self.pure(name, args, ctx, env)),
        }
    }

    fn aggregate(&self, name: &str, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<Value> {
        if args.len() != 1 {
            return err(format!("{name} takes one column"));
        }
        let (t, c) = self.col_arg(arg(args, 0, name)?, env, name)?;
        let rows = ctx.visible(self.m, t);
        let vals = self.column_values(t, c, &rows);
        match name {
            "COUNTA" => Ok(count(vals.filter(|v| !v.is_blank()).count())),
            "COUNTBLANK" => Ok(count(vals.filter(|v| v.is_blank()).count())),
            "DISTINCTCOUNT" | "DISTINCTCOUNTNOBLANK" => {
                let keys: HashSet<Key> = vals.filter(|v| name == "DISTINCTCOUNT" || !v.is_blank()).map(Value::key).collect();
                Ok(count(keys.len()))
            }
            _ => Ok(fold_numbers(name, vals.filter_map(Value::as_number))),
        }
    }

    fn iterate(&self, name: &str, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<Value> {
        let tv = self.table(arg(args, 0, name)?, ctx, env)?;
        let body = arg(args, 1, name)?;
        if name == "RANKX" {
            return self.rankx(&tv, body, ctx, env);
        }
        let mut vals = Vec::with_capacity(tv.len());
        for f in frames(&tv) {
            vals.push(self.scalar(body, ctx, &env.with_frame(f))?);
        }
        match name {
            "COUNTAX" => Ok(count(vals.iter().filter(|v| !v.is_blank()).count())),
            "COUNTX" => Ok(count(vals.iter().filter(|v| v.as_number().is_some()).count())),
            "CONCATENATEX" => {
                let delim = match args.get(2) {
                    Some(d) => self.scalar(d, ctx, env)?.to_text(),
                    None => String::new(),
                };
                Ok(Value::text(vals.iter().map(Value::to_text).collect::<Vec<_>>().join(&delim)))
            }
            _ => Ok(fold_numbers(name.strip_suffix('X').unwrap_or(name), vals.iter().filter_map(Value::as_number))),
        }
    }

    /// RANKX(table, expr): the rank of the current context's value of `expr` among the table's
    /// rows (descending, ties share a rank).
    fn rankx(&self, tv: &TableVal, body: &Expr, ctx: &Ctx, env: &Env) -> Result<Value> {
        let me = self.scalar(body, ctx, env)?;
        let mut higher = 0usize;
        for f in frames(tv) {
            let v = self.scalar(body, ctx, &env.with_frame(f))?;
            if compare(&v, &me).is_gt() {
                higher += 1;
            }
        }
        Ok(count(higher + 1))
    }

    /// CALCULATE's filter arguments, applied to `ctx` (after context transition when `transition`).
    fn filter_args(&self, args: &[Expr], ctx: &Ctx, env: &Env, transition: bool) -> Result<Ctx> {
        let mut new = if transition { self.transition(ctx, env) } else { ctx.clone() };
        let mut adds = Vec::new();
        for a in args {
            match a {
                Expr::Call(f, xs) if matches!(f.as_str(), "ALL" | "REMOVEFILTERS" | "ALLNOBLANKROW" | "ALLSELECTED") => {
                    self.remove_filters(&mut new, xs, env)?
                }
                Expr::Call(f, xs) if f == "ALLEXCEPT" => self.all_except(&mut new, xs, env)?,
                // Relationship direction changes aren't modelled: CROSSFILTER keeps the model's.
                Expr::Call(f, _) if f == "CROSSFILTER" => {}
                Expr::Call(f, xs) if f == "USERELATIONSHIP" => {
                    let a = self.col_arg(arg(xs, 0, f)?, env, f)?;
                    let b = self.col_arg(arg(xs, 1, f)?, env, f)?;
                    let i = self
                        .m
                        .relationships
                        .iter()
                        .position(|r| {
                            let ends = ((r.from_table, r.from_column), (r.to_table, r.to_column));
                            ends == (a, b) || ends == (b, a)
                        })
                        .ok_or_else(|| Error::Eval("USERELATIONSHIP: there is no relationship between those columns".into()))?;
                    new.use_rels.push(i);
                }
                Expr::Call(f, xs) if f == "KEEPFILTERS" => adds.push((self.filter_from(arg(xs, 0, f)?, ctx, env)?, true)),
                Expr::Call(f, xs) if f == "FILTERARG" => adds.push((self.filter_from(arg(xs, 0, f)?, ctx, env)?, false)),
                _ => adds.push((self.filter_from(a, ctx, env)?, false)),
            }
        }
        for ((filters, whole), keep) in adds {
            if let Some(t) = whole {
                new.remove_table(t);
            }
            for f in filters {
                if keep { new.keep(f) } else { new.replace(f) }
            }
        }
        Ok(new)
    }

    fn remove_filters(&self, ctx: &mut Ctx, xs: &[Expr], env: &Env) -> Result<()> {
        if xs.is_empty() {
            ctx.filters.clear();
        }
        for x in xs {
            match x {
                Expr::Name(n) => {
                    let t = self.m.table_index(n).ok_or_else(|| Error::Eval(format!("no table named `{n}`")))?;
                    ctx.remove_table(t);
                }
                other => {
                    let (t, c) = self.col_arg(other, env, "ALL")?;
                    ctx.remove_column(t, c);
                }
            }
        }
        Ok(())
    }

    fn all_except(&self, ctx: &mut Ctx, xs: &[Expr], env: &Env) -> Result<()> {
        let t = match arg(xs, 0, "ALLEXCEPT")? {
            Expr::Name(n) => self.m.table_index(n).ok_or_else(|| Error::Eval(format!("no table named `{n}`")))?,
            _ => return err("ALLEXCEPT needs a table first"),
        };
        let mut keep = Vec::new();
        for x in xs.get(1..).unwrap_or_default() {
            keep.push(self.col_arg(x, env, "ALLEXCEPT")?);
        }
        ctx.filters.retain(|f| match f {
            Filter::Values { table, column, .. } => *table != t || keep.contains(&(*table, *column)),
            Filter::Rows { table, .. } => *table != t,
        });
        Ok(())
    }

    /// The filters one CALCULATE argument stands for, and the table it clears first (time
    /// intelligence works on whole date tables).
    fn filter_from(&self, a: &Expr, ctx: &Ctx, env: &Env) -> Result<(Vec<Filter>, Option<usize>)> {
        if is_predicate(a) {
            return Ok((vec![self.predicate_filter(a, ctx, env)?], None));
        }
        let tv = self.table(a, ctx, env)?;
        let mut whole = None;
        let filters = match &*tv {
            TableVal::Rows { table, rows } => {
                let n = self.m.tables.get(*table).map(|t| t.row_count()).unwrap_or(0);
                vec![Filter::Rows { table: *table, rows: Rc::new(RowSet::from_rows(n, rows.iter().copied())) }]
            }
            TableVal::Values { columns, data } => {
                let mut out = Vec::new();
                for (j, cn) in columns.iter().enumerate() {
                    if let ColName::Model(t, c) = cn {
                        let keys: HashSet<Key> = data.iter().filter_map(|r| r.get(j)).map(Value::key).collect();
                        out.push(Filter::Values { table: *t, column: *c, keys: Rc::new(keys) });
                        if matches!(a, Expr::Call(f, _) if TIME_TABLE_FNS.contains(&f.as_str())) {
                            whole = Some(*t);
                        }
                    }
                }
                if out.is_empty() {
                    return err("a filter table must have at least one column of the model");
                }
                out
            }
        };
        Ok((filters, whole))
    }

    /// `Table[Column] = "x"` and other boolean conditions on one column, as a filter on that
    /// column's values (FILTER(ALL(Table[Column]), condition)).
    fn predicate_filter(&self, a: &Expr, ctx: &Ctx, env: &Env) -> Result<Filter> {
        let mut cols = Vec::new();
        collect_columns(a, &mut cols);
        cols.sort();
        cols.dedup();
        let (table, column) = match cols.as_slice() {
            [(t, c)] => self.m.column_ref(t, c)?,
            [] => return err("a filter condition must refer to a column; use FILTER(...) for other conditions"),
            _ => return err("a filter condition may refer to one column only; use FILTER(...) for conditions on several columns"),
        };
        let Some(col) = self.m.tables.get(table).and_then(|t| t.column(column)) else { return err("no such column") };
        let columns = Rc::new(vec![ColName::Model(table, column)]);
        let mut keys = HashSet::new();
        for id in 0..col.cardinality() as u32 {
            let v = col.dict_value(id).clone();
            let f = Frame::Tuple { columns: columns.clone(), values: Rc::new(vec![v.clone()]) };
            if self.scalar(a, ctx, &env.with_frame(f))?.truthy()? {
                keys.insert(v.key());
            }
        }
        Ok(Filter::Values { table, column, keys: Rc::new(keys) })
    }

    fn filter_fn(&self, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<TableVal> {
        let tv = self.table(arg(args, 0, "FILTER")?, ctx, env)?;
        let cond = arg(args, 1, "FILTER")?;
        let mut keep = Vec::new();
        for (i, f) in frames(&tv).into_iter().enumerate() {
            if self.scalar(cond, ctx, &env.with_frame(f))?.truthy()? {
                keep.push(i);
            }
        }
        Ok(tv.select(&keep))
    }

    fn all_fn(&self, args: &[Expr], env: &Env) -> Result<TableVal> {
        if let [Expr::Name(n)] = args {
            let t = self.m.table_index(n).ok_or_else(|| Error::Eval(format!("no table named `{n}`")))?;
            let rows = self.m.tables.get(t).map(|tb| tb.row_count()).unwrap_or(0) as u32;
            return Ok(TableVal::Rows { table: t, rows: (0..rows).collect() });
        }
        let mut cols = Vec::new();
        for a in args {
            cols.push(self.col_arg(a, env, "ALL")?);
        }
        let t = cols.first().map(|c| c.0).ok_or_else(|| Error::Eval("ALL needs a table or columns".into()))?;
        if cols.iter().any(|c| c.0 != t) {
            return err("ALL: the columns must come from one table");
        }
        let n = self.m.tables.get(t).map(|tb| tb.row_count()).unwrap_or(0);
        let idx: Vec<usize> = cols.iter().map(|c| c.1).collect();
        let data = self.distinct_tuples(t, &idx, 0..n)?;
        Ok(TableVal::Values { columns: cols.iter().map(|(t, c)| ColName::Model(*t, *c)).collect(), data })
    }

    fn values_fn(&self, name: &str, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<TableVal> {
        match arg(args, 0, name)? {
            Expr::Name(n) if self.m.table_index(n).is_some() && !env.vars.iter().any(|(v, _)| v.eq_ignore_ascii_case(n)) => {
                let t = self.m.table_index(n).unwrap_or_default();
                Ok(self.visible_rows(t, ctx))
            }
            e @ (Expr::Column { .. } | Expr::Bracket(_)) => {
                let (t, c) = self.col_arg(e, env, name)?;
                let rows = ctx.visible(self.m, t);
                let data = self.distinct_tuples(t, &[c], rows.iter())?;
                Ok(TableVal::Values { columns: vec![ColName::Model(t, c)], data })
            }
            other => {
                let tv = self.table(other, ctx, env)?;
                let mut seen = HashSet::new();
                let mut keep = Vec::new();
                for (i, f) in frames(&tv).into_iter().enumerate() {
                    let key: Vec<Key> = match &f {
                        Frame::Row { table, row } => {
                            self.m.tables.get(*table).map(|t| t.columns.iter().map(|c| c.value(*row as usize).key()).collect()).unwrap_or_default()
                        }
                        Frame::Tuple { values, .. } => values.iter().map(Value::key).collect(),
                    };
                    if seen.insert(key) {
                        keep.push(i);
                    }
                }
                Ok(tv.select(&keep))
            }
        }
    }

    /// SUMMARIZECOLUMNS(group columns…, filter tables…, "Name", expression, …).
    fn summarize_columns(&self, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<TableVal> {
        let mut groups = Vec::new();
        let mut filters = Vec::new();
        let mut measures = Vec::new();
        let mut i = 0;
        while let Some(a) = args.get(i) {
            match a {
                Expr::Column { .. } => groups.push(self.col_arg(a, env, "SUMMARIZECOLUMNS")?),
                Expr::Text(n) => {
                    measures.push((n.clone(), arg(args, i + 1, "SUMMARIZECOLUMNS")?));
                    i += 1;
                }
                other => filters.push(other.clone()),
            }
            i += 1;
        }
        let base = self.filter_args(&filters, ctx, env, false)?;
        self.group_eval(&base, &groups, &measures, env, None)
    }

    /// One row per existing combination of `groups` (combinations within a table come from its
    /// visible rows; tables are cross-joined), with `measures` evaluated for each; rows whose
    /// measures are all blank are dropped. `tuples` gives the combinations directly.
    fn group_eval(
        &self,
        base: &Ctx,
        groups: &[(usize, usize)],
        measures: &[(String, &Expr)],
        env: &Env,
        tuples: Option<Vec<Vec<Value>>>,
    ) -> Result<TableVal> {
        let combos = match tuples {
            Some(t) => t,
            None => self.group_combos(base, groups)?,
        };
        let inner = env.without_rows();
        let mut data = Vec::with_capacity(combos.len());
        for combo in combos {
            let mut c = base.clone();
            for ((t, col), v) in groups.iter().zip(combo.iter()) {
                c.replace(Filter::Values { table: *t, column: *col, keys: Rc::new(HashSet::from([v.key()])) });
            }
            let mut row = combo;
            let mut any = measures.is_empty();
            for (_, e) in measures {
                let v = self.scalar(e, &c, &inner)?;
                any |= !v.is_blank();
                row.push(v);
            }
            if any {
                data.push(row);
            }
        }
        let mut columns: Vec<ColName> = groups.iter().map(|(t, c)| ColName::Model(*t, *c)).collect();
        columns.extend(measures.iter().map(|(n, _)| ColName::Named(n.clone())));
        Ok(TableVal::Values { columns, data })
    }

    fn group_combos(&self, base: &Ctx, groups: &[(usize, usize)]) -> Result<Vec<Vec<Value>>> {
        // Tables in first-mention order, with the positions of their columns in `groups`.
        let mut by_table: Vec<(usize, Vec<usize>)> = Vec::new();
        for (pos, (t, _)) in groups.iter().enumerate() {
            match by_table.iter_mut().find(|(bt, _)| bt == t) {
                Some((_, ps)) => ps.push(pos),
                None => by_table.push((*t, vec![pos])),
            }
        }
        let mut combos: Vec<Vec<Option<Value>>> = vec![vec![None; groups.len()]];
        for (t, positions) in &by_table {
            let cols: Vec<usize> = positions.iter().filter_map(|p| groups.get(*p).map(|g| g.1)).collect();
            let rows = base.visible(self.m, *t);
            let tuples = self.distinct_tuples(*t, &cols, rows.iter())?;
            if combos.len().saturating_mul(tuples.len()) > MAX_TABLE_ROWS {
                return err("the result has too many rows");
            }
            let mut next = Vec::with_capacity(combos.len() * tuples.len());
            for c in &combos {
                for tup in &tuples {
                    let mut c2 = c.clone();
                    for (p, v) in positions.iter().zip(tup.iter()) {
                        if let Some(slot) = c2.get_mut(*p) {
                            *slot = Some(v.clone());
                        }
                    }
                    next.push(c2);
                }
            }
            combos = next;
        }
        let mut out: Vec<Vec<Value>> = combos.into_iter().map(|c| c.into_iter().map(Option::unwrap_or_default).collect()).collect();
        out.sort_by(|a, b| a.iter().zip(b.iter()).map(|(x, y)| sort_order(x, y)).find(|o| o.is_ne()).unwrap_or(std::cmp::Ordering::Equal));
        Ok(out)
    }

    /// SUMMARIZE(table, group columns…, "Name", expression, …): group columns may come from the
    /// table or from tables it looks up (many-to-one).
    fn summarize(&self, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<TableVal> {
        let tv = self.table(arg(args, 0, "SUMMARIZE")?, ctx, env)?;
        let mut groups = Vec::new();
        let mut measures = Vec::new();
        let mut i = 1;
        while let Some(a) = args.get(i) {
            match a {
                Expr::Text(n) => {
                    measures.push((n.clone(), arg(args, i + 1, "SUMMARIZE")?));
                    i += 1;
                }
                other => groups.push(self.col_arg(other, env, "SUMMARIZE")?),
            }
            i += 1;
        }
        let mut seen = HashSet::new();
        let mut tuples = Vec::new();
        for f in frames(&tv) {
            let env2 = env.with_frame(f.clone());
            let mut tuple = Vec::with_capacity(groups.len());
            for (t, c) in &groups {
                let v = match self.row_value(*t, *c, &env2) {
                    Ok(v) => v,
                    Err(_) => self.related_value(*t, *c, &env2).unwrap_or_default(),
                };
                tuple.push(v);
            }
            if seen.insert(tuple.iter().map(Value::key).collect::<Vec<_>>()) {
                tuples.push(tuple);
            }
        }
        tuples.sort_by(|a, b| a.iter().zip(b.iter()).map(|(x, y)| sort_order(x, y)).find(|o| o.is_ne()).unwrap_or(std::cmp::Ordering::Equal));
        self.group_eval(ctx, &groups, &measures, env, Some(tuples))
    }

    fn add_columns(&self, name: &str, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<TableVal> {
        let tv = self.table(arg(args, 0, name)?, ctx, env)?;
        let mut new_cols = Vec::new();
        let mut i = 1;
        while let Some(a) = args.get(i) {
            let n = match a {
                Expr::Text(n) => n.clone(),
                _ => return err(format!("{name} takes pairs of \"Name\", expression")),
            };
            new_cols.push((n, arg(args, i + 1, name)?));
            i += 2;
        }
        let keep_base = name == "ADDCOLUMNS";
        let mut columns = Vec::new();
        if keep_base {
            columns = match &*tv {
                TableVal::Rows { table, .. } => {
                    (0..self.m.tables.get(*table).map(|t| t.columns.len()).unwrap_or(0)).map(|c| ColName::Model(*table, c)).collect()
                }
                TableVal::Values { columns, .. } => columns.clone(),
            };
        }
        columns.extend(new_cols.iter().map(|(n, _)| ColName::Named(n.clone())));
        let mut data = Vec::with_capacity(tv.len());
        for f in frames(&tv) {
            let mut row = if keep_base {
                match &f {
                    Frame::Row { table, row } => {
                        self.m.tables.get(*table).map(|t| t.columns.iter().map(|c| c.value(*row as usize).clone()).collect()).unwrap_or_default()
                    }
                    Frame::Tuple { values, .. } => (**values).clone(),
                }
            } else {
                Vec::new()
            };
            let env2 = env.with_frame(f);
            for (_, e) in &new_cols {
                row.push(self.scalar(e, ctx, &env2)?);
            }
            data.push(row);
        }
        Ok(TableVal::Values { columns, data })
    }

    fn topn(&self, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<TableVal> {
        let n = self.scalar(arg(args, 0, "TOPN")?, ctx, env)?.to_number()?.max(0.0) as usize;
        let tv = self.table(arg(args, 1, "TOPN")?, ctx, env)?;
        let mut keyed = Vec::with_capacity(tv.len());
        for (i, f) in frames(&tv).into_iter().enumerate() {
            let env2 = env.with_frame(f);
            let mut keys = Vec::new();
            let mut j = 2;
            while let Some(e) = args.get(j) {
                let asc = match args.get(j + 1) {
                    Some(o) => is_ascending(o),
                    None => false,
                };
                keys.push((self.scalar(e, ctx, &env2)?, asc));
                j += 2;
            }
            keyed.push((keys, i));
        }
        keyed.sort_by(|a, b| {
            a.0.iter()
                .zip(b.0.iter())
                .map(|((x, asc), (y, _))| if *asc { sort_order(x, y) } else { sort_order(y, x) })
                .find(|o| o.is_ne())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let keep: Vec<usize> = keyed.into_iter().take(n).map(|(_, i)| i).collect();
        Ok(tv.select(&keep))
    }

    fn series(&self, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<TableVal> {
        let start = self.scalar(arg(args, 0, "GENERATESERIES")?, ctx, env)?.to_number()?;
        let end = self.scalar(arg(args, 1, "GENERATESERIES")?, ctx, env)?.to_number()?;
        let step = match args.get(2) {
            Some(e) => self.scalar(e, ctx, env)?.to_number()?,
            None => 1.0,
        };
        if step <= 0.0 || !step.is_finite() || !start.is_finite() || !end.is_finite() {
            return err("GENERATESERIES needs a positive step");
        }
        let count = ((end - start) / step).floor() + 1.0;
        if count > MAX_TABLE_ROWS as f64 {
            return err("the result has too many rows");
        }
        let data = (0..count.max(0.0) as usize).map(|i| vec![Value::Number(start + step * i as f64)]).collect();
        Ok(TableVal::Values { columns: vec![ColName::Named("Value".into())], data })
    }

    /// RELATED(Table[Column]): the column's value in the row the current row looks up.
    fn related(&self, args: &[Expr], env: &Env) -> Result<Value> {
        let (t, c) = self.col_arg(arg(args, 0, "RELATED")?, env, "RELATED")?;
        self.related_value(t, c, env)
    }

    fn related_value(&self, t: usize, c: usize, env: &Env) -> Result<Value> {
        for f in env.frames.iter().rev() {
            if let Frame::Row { table, row } = f {
                if *table == t {
                    return self.row_value(t, c, env);
                }
                let mut visited = vec![false; self.m.tables.len()];
                if let Some(r) = self.related_row(*table, *row, t, &mut visited) {
                    return Ok(self.m.tables.get(t).and_then(|tb| tb.column(c)).map(|col| col.value(r as usize).clone()).unwrap_or_default());
                }
                if self.reachable(*table, t) {
                    return Ok(Value::Blank);
                }
            }
        }
        err(format!("RELATED: {} isn't on the lookup side of a relationship from the current row's table", self.m.column_name(t, c)))
    }

    /// The row of `target` that row `row` of `from` looks up, following many-to-one relationships.
    fn related_row(&self, from: usize, row: u32, target: usize, visited: &mut Vec<bool>) -> Option<u32> {
        if let Some(v) = visited.get_mut(from) {
            *v = true;
        }
        for r in self.m.relationships.iter().filter(|r| r.active && r.from_table == from) {
            if visited.get(r.to_table).copied().unwrap_or(true) {
                continue;
            }
            let key = self.m.tables.get(from)?.column(r.from_column)?.value(row as usize).key();
            let Some(to_row) = r.lookup(&key) else { continue };
            if r.to_table == target {
                return Some(to_row);
            }
            if let Some(found) = self.related_row(r.to_table, to_row, target, visited) {
                return Some(found);
            }
        }
        None
    }

    fn reachable(&self, from: usize, target: usize) -> bool {
        let mut stack = vec![from];
        let mut seen = vec![false; self.m.tables.len()];
        while let Some(t) = stack.pop() {
            if t == target {
                return true;
            }
            if seen.get(t).copied().unwrap_or(true) {
                continue;
            }
            if let Some(s) = seen.get_mut(t) {
                *s = true;
            }
            stack.extend(self.m.relationships.iter().filter(|r| r.active && r.from_table == t).map(|r| r.to_table));
        }
        false
    }

    /// LOOKUPVALUE(result column, search column, value, …).
    fn lookup_value(&self, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<Value> {
        let (t, c) = self.col_arg(arg(args, 0, "LOOKUPVALUE")?, env, "LOOKUPVALUE")?;
        let mut conds = Vec::new();
        let mut i = 1;
        while let (Some(col), Some(val)) = (args.get(i), args.get(i + 1)) {
            let (st, sc) = self.col_arg(col, env, "LOOKUPVALUE")?;
            if st != t {
                return err("LOOKUPVALUE: the search columns must be in the result column's table");
            }
            conds.push((sc, self.scalar(val, ctx, env)?));
            i += 2;
        }
        let Some(tb) = self.m.tables.get(t) else { return Ok(Value::Blank) };
        let mut found: Option<Value> = None;
        for r in 0..tb.row_count() {
            if conds.iter().all(|(sc, v)| tb.column(*sc).is_some_and(|col| compare(col.value(r), v).is_eq())) {
                let v = tb.column(c).map(|col| col.value(r).clone()).unwrap_or_default();
                match &found {
                    Some(prev) if prev.key() != v.key() => return err("LOOKUPVALUE: several different values match"),
                    _ => found = Some(v),
                }
            }
        }
        match (found, args.get(i)) {
            (Some(v), _) => Ok(v),
            (None, Some(alt)) => self.scalar(alt, ctx, env),
            (None, None) => Ok(Value::Blank),
        }
    }

    fn filter_info(&self, name: &str, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<Value> {
        let a = arg(args, 0, name)?;
        if let Expr::Name(n) = a
            && let Some(t) = self.m.table_index(n)
        {
            return match name {
                "ISFILTERED" => Ok(Value::Bool(ctx.is_filtered(t, None))),
                "ISCROSSFILTERED" => Ok(Value::Bool(!ctx.visible(self.m, t).is_full())),
                _ => err(format!("{name} needs a column")),
            };
        }
        let (t, c) = self.col_arg(a, env, name)?;
        let rows = ctx.visible(self.m, t);
        match name {
            "ISFILTERED" => Ok(Value::Bool(ctx.is_filtered(t, Some(c)))),
            "ISCROSSFILTERED" => Ok(Value::Bool(!rows.is_full())),
            _ => {
                let mut distinct =
                    self.column_values(t, c, &rows).map(|v| (v.key(), v.clone())).collect::<std::collections::HashMap<_, _>>().into_values();
                let first = distinct.next();
                let one = first.is_some() && distinct.next().is_none();
                match (name, one) {
                    ("HASONEVALUE", one) => Ok(Value::Bool(one)),
                    (_, true) => Ok(first.unwrap_or_default()),
                    (_, false) => match args.get(1) {
                        Some(alt) => self.scalar(alt, ctx, env),
                        None => Ok(Value::Blank),
                    },
                }
            }
        }
    }

    // ------------------------------------------------------------ time intelligence

    fn time_table(&self, name: &str, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<TableVal> {
        let (t, c) = self.col_arg(arg(args, 0, name)?, env, name)?;
        let Some(col) = self.m.tables.get(t).and_then(|tb| tb.column(c)) else { return err("no such column") };
        let all: Vec<f64> = (0..col.cardinality() as u32).filter_map(|id| col.dict_value(id).as_number()).collect();
        let rows = ctx.visible(self.m, t);
        let mut cur: Vec<f64> = self.column_values(t, c, &rows).filter_map(Value::as_number).collect();
        cur.sort_by(f64::total_cmp);
        cur.dedup();
        let (Some(&lo), Some(&hi)) = (cur.first(), cur.last()) else {
            return Ok(TableVal::Values { columns: vec![ColName::Model(t, c)], data: vec![] });
        };
        let picked: Vec<f64> = match name {
            "DATESYTD" | "DATESQTD" | "DATESMTD" => {
                let start = crate::dates::period_start(hi, name).ok_or_else(|| Error::Eval("not a date".into()))?;
                all.iter().copied().filter(|d| *d >= start && *d <= hi).collect()
            }
            "DATESBETWEEN" => {
                let a = self.scalar(arg(args, 1, name)?, ctx, env)?;
                let b = self.scalar(arg(args, 2, name)?, ctx, env)?;
                let a = if a.is_blank() { f64::NEG_INFINITY } else { a.to_number()? };
                let b = if b.is_blank() { f64::INFINITY } else { b.to_number()? };
                all.iter().copied().filter(|d| *d >= a && *d <= b).collect()
            }
            "PREVIOUSYEAR" | "PREVIOUSQUARTER" | "PREVIOUSMONTH" => {
                let (a, b) = crate::dates::previous_period(lo, name).ok_or_else(|| Error::Eval("not a date".into()))?;
                all.iter().copied().filter(|d| *d >= a && *d <= b).collect()
            }
            _ => {
                let (n, unit) = if name == "SAMEPERIODLASTYEAR" {
                    (-1, "YEAR".to_string())
                } else {
                    let n = self.scalar(arg(args, 1, name)?, ctx, env)?.to_number()? as i32;
                    let unit = match arg(args, 2, name)? {
                        Expr::Name(u) => u.to_ascii_uppercase(),
                        other => self.scalar(other, ctx, env)?.to_text().to_ascii_uppercase(),
                    };
                    (n, unit)
                };
                let exists: HashSet<u64> = all.iter().map(|d| d.to_bits()).collect();
                cur.iter().filter_map(|d| crate::dates::shift(*d, n, &unit)).filter(|d| exists.contains(&d.to_bits())).collect()
            }
        };
        let mut picked = picked;
        picked.sort_by(f64::total_cmp);
        Ok(TableVal::Values { columns: vec![ColName::Model(t, c)], data: picked.into_iter().map(|d| vec![Value::Number(d)]).collect() })
    }

    // ------------------------------------------------------------ scalar functions

    fn pure(&self, name: &str, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<Value> {
        // Functions that evaluate only some of their arguments.
        match name {
            "IF" => {
                let c = self.scalar(arg(args, 0, name)?, ctx, env)?.truthy()?;
                return match (c, args.get(1), args.get(2)) {
                    (true, Some(a), _) => self.scalar(a, ctx, env),
                    (false, _, Some(b)) => self.scalar(b, ctx, env),
                    _ => Ok(Value::Blank),
                };
            }
            "SWITCH" => return self.switch(args, ctx, env),
            "IFERROR" => {
                return match self.scalar(arg(args, 0, name)?, ctx, env) {
                    Ok(v) if v.as_number().is_none_or(f64::is_finite) => Ok(v),
                    _ => self.scalar(arg(args, 1, name)?, ctx, env),
                };
            }
            "COALESCE" => {
                for a in args {
                    let v = self.scalar(a, ctx, env)?;
                    if !v.is_blank() {
                        return Ok(v);
                    }
                }
                return Ok(Value::Blank);
            }
            "ISEMPTY" => return Ok(Value::Bool(self.table(arg(args, 0, name)?, ctx, env)?.is_empty())),
            _ => {}
        }
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(self.scalar(a, ctx, env)?);
        }
        crate::functions::call(name, &vals)
    }

    fn switch(&self, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<Value> {
        let x = self.scalar(arg(args, 0, "SWITCH")?, ctx, env)?;
        let mut i = 1;
        while let (Some(v), Some(r)) = (args.get(i), args.get(i + 1)) {
            if compare(&x, &self.scalar(v, ctx, env)?).is_eq() {
                return self.scalar(r, ctx, env);
            }
            i += 2;
        }
        match args.get(i) {
            Some(e) => self.scalar(e, ctx, env),
            None => Ok(Value::Blank),
        }
    }
}

fn count(n: usize) -> Value {
    if n == 0 { Value::Blank } else { Value::Number(n as f64) }
}

fn fold_numbers(name: &str, it: impl Iterator<Item = f64>) -> Value {
    let v: Vec<f64> = it.collect();
    if v.is_empty() {
        return Value::Blank;
    }
    let n = v.len() as f64;
    Value::Number(match name {
        "SUM" => v.iter().sum(),
        "AVERAGE" => v.iter().sum::<f64>() / n,
        "MIN" => v.iter().copied().fold(f64::INFINITY, f64::min),
        "MAX" => v.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        "PRODUCT" => v.iter().product(),
        "MEDIAN" => {
            let mut s = v.clone();
            s.sort_by(f64::total_cmp);
            let mid = s.len() / 2;
            if s.len().is_multiple_of(2) {
                (s.get(mid - 1).copied().unwrap_or(0.0) + s.get(mid).copied().unwrap_or(0.0)) / 2.0
            } else {
                s.get(mid).copied().unwrap_or(0.0)
            }
        }
        _ => n,
    })
}

fn is_ascending(e: &Expr) -> bool {
    match e {
        Expr::Name(n) => n.eq_ignore_ascii_case("ASC"),
        Expr::Number(n) => *n != 0.0,
        Expr::Bool(b) => *b,
        _ => false,
    }
}

fn is_predicate(e: &Expr) -> bool {
    match e {
        Expr::Binary(op, _, _) => !matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow | BinOp::Concat),
        Expr::Unary(UnOp::Not, _) => true,
        Expr::Call(f, _) => matches!(f.as_str(), "ISBLANK" | "CONTAINSSTRING" | "AND" | "OR" | "NOT"),
        _ => false,
    }
}

fn collect_columns(e: &Expr, out: &mut Vec<(String, String)>) {
    match e {
        Expr::Column { table, column } => out.push((table.to_lowercase(), column.to_lowercase())),
        Expr::Unary(_, x) => collect_columns(x, out),
        Expr::Binary(_, a, b) => {
            collect_columns(a, out);
            collect_columns(b, out);
        }
        Expr::Call(_, args) => args.iter().for_each(|a| collect_columns(a, out)),
        Expr::Var(bs, body) => {
            bs.iter().for_each(|(_, x)| collect_columns(x, out));
            collect_columns(body, out);
        }
        Expr::TableCtor(rows) => rows.iter().flatten().for_each(|x| collect_columns(x, out)),
        _ => {}
    }
}

fn neg(v: &Value) -> Result<Value> {
    if v.is_blank() { Ok(Value::Blank) } else { Ok(Value::Number(-v.to_number()?)) }
}

/// Arithmetic and comparison on two values, with DAX's blank rules (BLANK + BLANK is BLANK,
/// BLANK * x is BLANK, BLANK = 0 is TRUE).
pub fn binary_scalar(op: BinOp, a: &Value, b: &Value) -> Result<Value> {
    let both_blank = a.is_blank() && b.is_blank();
    Ok(match op {
        BinOp::Add | BinOp::Sub if both_blank => Value::Blank,
        BinOp::Add => Value::Number(a.to_number()? + b.to_number()?),
        BinOp::Sub => Value::Number(a.to_number()? - b.to_number()?),
        BinOp::Mul if a.is_blank() || b.is_blank() => Value::Blank,
        BinOp::Mul => Value::Number(a.to_number()? * b.to_number()?),
        BinOp::Div if a.is_blank() => Value::Blank,
        BinOp::Div => Value::Number(a.to_number()? / b.to_number()?),
        BinOp::Pow if both_blank => Value::Blank,
        BinOp::Pow => Value::Number(a.to_number()?.powf(b.to_number()?)),
        BinOp::Concat => Value::text(format!("{}{}", a.to_text(), b.to_text())),
        BinOp::StrictEq if a.is_blank() || b.is_blank() => Value::Bool(both_blank),
        BinOp::Eq | BinOp::StrictEq => Value::Bool(compare(a, b).is_eq()),
        BinOp::Ne => Value::Bool(compare(a, b).is_ne()),
        BinOp::Lt => Value::Bool(compare(a, b).is_lt()),
        BinOp::Le => Value::Bool(compare(a, b).is_le()),
        BinOp::Gt => Value::Bool(compare(a, b).is_gt()),
        BinOp::Ge => Value::Bool(compare(a, b).is_ge()),
        BinOp::And => Value::Bool(a.truthy()? && b.truthy()?),
        BinOp::Or => Value::Bool(a.truthy()? || b.truthy()?),
        BinOp::In => return err("IN needs a table on its right"),
    })
}
