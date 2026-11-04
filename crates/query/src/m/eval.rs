//! The M evaluator.
//!
//! `let` bindings are lazy (each is evaluated once, when first used), so a query's unused steps
//! cost nothing and steps may refer to each other in any order. Functions are closures
//! (`each`, `(x) => …`), library functions (by name, see [`crate::funcs`]) or native functions
//! made by library functions (`Splitter.SplitTextByDelimiter(",")`).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use super::MAX_DEPTH;
use super::ast::{BinOp, Expr, ListItem, UnOp};
use crate::value::{MType, Record, Table, Value, compare, values_equal};
use crate::{Error, Result};

/// What the queries reach outside themselves: other queries and parameters, the workbook's
/// tables, files and folders.
pub trait Host {
    /// Another query or a parameter, by name.
    fn resolve(&self, _name: &str) -> Option<Result<Value>> {
        None
    }
    /// `Excel.CurrentWorkbook()`: a table with `Name` and `Content` columns.
    fn current_workbook(&self) -> Result<Table> {
        Err(Error::Eval("This query can't read the current workbook here.".into()))
    }
    fn file_contents(&self, path: &str) -> Result<Vec<u8>> {
        Err(Error::Eval(format!("Files can't be read here ({path}).")))
    }
    /// `Excel.Workbook(binary)`: `Name`, `Data`, `Item`, `Kind`, `Hidden`.
    fn excel_workbook(&self, _bytes: &[u8], _use_headers: bool) -> Result<Table> {
        Err(Error::Eval("Excel workbooks can't be read here.".into()))
    }
    /// `Folder.Files(path)`: `Content`, `Name`, `Extension`, `Date modified`, `Folder Path`.
    fn folder_files(&self, path: &str) -> Result<Table> {
        Err(Error::Eval(format!("Folders can't be read here ({path}).")))
    }
    fn web_contents(&self, url: &str) -> Result<Vec<u8>> {
        Err(Error::Eval(format!("Web sources aren't supported yet ({url}).")))
    }
    /// Database sources (`Sqlite.Database`, `PostgreSQL.Database`, `MySQL.Database`,
    /// `Sql.Database`, `Odbc.Query`, `Parquet.Document`, …): the connector `kind`, the
    /// function's text arguments, and the `Query` option when one is given.
    fn database(&self, kind: &str, args: &[String], query: Option<&str>) -> Result<Table> {
        let _ = (args, query);
        Err(Error::Eval(format!("The {kind} connector isn't available here.")))
    }
    /// `VizCraft.Sql(sql)` / `Sql.Native(sql)`: runs SQL on the host's embedded analytical
    /// engine (DuckDB over the model tables and the project warehouse).
    fn sql(&self, query: &str) -> Result<Table> {
        let _ = query;
        Err(Error::Eval("The SQL engine isn't available here (VizCraft.Sql runs in the desktop app).".into()))
    }
}

/// A host with no outside world (tests, formulas that need nothing).
pub struct NoHost;
impl Host for NoHost {}

pub type NativeFn = dyn Fn(&Evaluator, Vec<Value>) -> Result<Value>;

pub enum Function {
    /// A library function, by name.
    Builtin(String),
    Closure {
        params: Vec<String>,
        body: Rc<Expr>,
        env: Env,
    },
    Native(Rc<NativeFn>),
}

impl std::fmt::Debug for Function {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Function::Builtin(n) => write!(f, "Builtin({n})"),
            Function::Closure { params, .. } => write!(f, "Closure({params:?})"),
            Function::Native(_) => write!(f, "Native"),
        }
    }
}

enum LazyState {
    Pending(Expr, Weak<Scope>),
    Busy,
    Done(Result<Value>),
}

pub struct Lazy(RefCell<LazyState>);

impl Lazy {
    fn done(v: Value) -> Rc<Lazy> {
        Rc::new(Lazy(RefCell::new(LazyState::Done(Ok(v)))))
    }
}

pub struct Scope {
    vars: Vec<(String, Rc<Lazy>)>,
    parent: Env,
}

#[derive(Clone, Default)]
pub struct Env(Option<Rc<Scope>>);

impl Env {
    pub fn with(&self, vars: Vec<(String, Value)>) -> Env {
        Env(Some(Rc::new(Scope { vars: vars.into_iter().map(|(n, v)| (n, Lazy::done(v))).collect(), parent: self.clone() })))
    }

    fn find(&self, name: &str) -> Option<Rc<Lazy>> {
        let mut cur = self.0.clone();
        while let Some(s) = cur {
            if let Some((_, l)) = s.vars.iter().rev().find(|(n, _)| n == name) {
                return Some(l.clone());
            }
            cur = s.parent.0.clone();
        }
        None
    }
}

/// Evaluation steps one query may take before it is stopped.
const MAX_STEPS: u64 = 2_000_000_000;

pub struct Evaluator<'h> {
    pub host: &'h dyn Host,
    depth: Cell<usize>,
    steps: Cell<u64>,
}

fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Eval(msg.into()))
}

/// The value itself, or its error as `Err`.
pub fn check(v: Value) -> Result<Value> {
    match v {
        Value::Error(m) => Err(Error::Eval(m.to_string())),
        v => Ok(v),
    }
}

impl<'h> Evaluator<'h> {
    pub fn new(host: &'h dyn Host) -> Evaluator<'h> {
        Evaluator { host, depth: Cell::new(0), steps: Cell::new(0) }
    }

    pub fn tick(&self) -> Result<()> {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        if n > MAX_STEPS { err("The query is too large to finish.") } else { Ok(()) }
    }

    pub fn eval(&self, e: &Expr, env: &Env) -> Result<Value> {
        let d = self.depth.get() + 1;
        if d > MAX_DEPTH * 4 {
            return err("The evaluation nested too deeply (a query or function calls itself?).");
        }
        self.depth.set(d);
        let r = self.eval_inner(e, env);
        self.depth.set(d - 1);
        r
    }

    fn eval_inner(&self, e: &Expr, env: &Env) -> Result<Value> {
        self.tick()?;
        Ok(match e {
            Expr::Null => Value::Null,
            Expr::Bool(b) => Value::Logical(*b),
            Expr::Num(n) => Value::Number(*n),
            Expr::Text(t) => Value::text(t),
            Expr::Type(t) => Value::Type(*t),
            Expr::Ident(n) => self.lookup(n, env)?,
            Expr::Let(bindings, body) => {
                let scope = Rc::new_cyclic(|weak: &Weak<Scope>| Scope {
                    vars: bindings
                        .iter()
                        .map(|(n, x)| (n.clone(), Rc::new(Lazy(RefCell::new(LazyState::Pending(x.clone(), weak.clone()))))))
                        .collect(),
                    parent: env.clone(),
                });
                self.eval(body, &Env(Some(scope)))?
            }
            Expr::If(c, t, f) => match check(self.eval(c, env)?)? {
                Value::Logical(true) => self.eval(t, env)?,
                Value::Logical(false) => self.eval(f, env)?,
                other => return err(format!("We cannot convert the value {} to type Logical.", other.display())),
            },
            Expr::Lambda(params, body) => {
                Value::Function(Rc::new(Function::Closure { params: params.clone(), body: Rc::new((**body).clone()), env: env.clone() }))
            }
            Expr::Call(f, args) => {
                let fv = check(self.eval(f, env)?)?;
                let mut vals = Vec::with_capacity(args.len());
                for a in args {
                    vals.push(self.eval(a, env)?);
                }
                self.apply(&fv, vals)?
            }
            Expr::Field(target, name, opt) => {
                let t = match target {
                    Some(t) => check(self.eval(t, env)?)?,
                    None => self.lookup("_", env)?,
                };
                field(&t, name, *opt)?
            }
            Expr::Project(target, names, opt) => {
                let t = check(self.eval(target, env)?)?;
                project(&t, names, *opt)?
            }
            Expr::Item(target, idx, opt) => {
                let t = check(self.eval(target, env)?)?;
                let i = check(self.eval(idx, env)?)?;
                item(&t, &i, *opt)?
            }
            Expr::Record(fields) => {
                let mut r = Record::default();
                for (n, x) in fields {
                    r.names.push(n.clone());
                    r.values.push(self.eval(x, env)?);
                }
                Value::record(r)
            }
            Expr::List(items) => {
                let mut out = Vec::new();
                for it in items {
                    match it {
                        ListItem::One(x) => out.push(self.eval(x, env)?),
                        ListItem::Range(a, b) => {
                            let (av, bv) = (check(self.eval(a, env)?)?, check(self.eval(b, env)?)?);
                            if let (Some(x), Some(y)) = (av.as_text().and_then(|t| t.chars().next()), bv.as_text().and_then(|t| t.chars().next())) {
                                out.extend((x..=y).take(1_000_000).map(|c| Value::text(c.to_string())));
                                continue;
                            }
                            let a = to_int(&av)?;
                            let b = to_int(&bv)?;
                            if b.saturating_sub(a) > 50_000_000 {
                                return err("The list is too long.");
                            }
                            out.extend((a..=b).map(|n| Value::Number(n as f64)));
                        }
                    }
                }
                Value::list(out)
            }
            Expr::Unary(op, x) => {
                let v = check(self.eval(x, env)?)?;
                match (op, v) {
                    (_, Value::Null) => Value::Null,
                    (UnOp::Neg, Value::Number(n)) => Value::Number(-n),
                    (UnOp::Neg, Value::Duration(n)) => Value::Duration(-n),
                    (UnOp::Plus, v @ Value::Number(_)) => v,
                    (UnOp::Not, Value::Logical(b)) => Value::Logical(!b),
                    (_, v) => return err(format!("The operator can't be applied to a {} value.", v.kind())),
                }
            }
            Expr::Binary(BinOp::And, a, b) => match check(self.eval(a, env)?)? {
                Value::Logical(false) => Value::Logical(false),
                Value::Logical(true) => logical(check(self.eval(b, env)?)?)?,
                Value::Null => match check(self.eval(b, env)?)? {
                    Value::Logical(false) => Value::Logical(false),
                    _ => Value::Null,
                },
                v => return err(format!("We cannot convert the value {} to type Logical.", v.display())),
            },
            Expr::Binary(BinOp::Or, a, b) => match check(self.eval(a, env)?)? {
                Value::Logical(true) => Value::Logical(true),
                Value::Logical(false) => logical(check(self.eval(b, env)?)?)?,
                Value::Null => match check(self.eval(b, env)?)? {
                    Value::Logical(true) => Value::Logical(true),
                    _ => Value::Null,
                },
                v => return err(format!("We cannot convert the value {} to type Logical.", v.display())),
            },
            Expr::Binary(BinOp::Coalesce, a, b) => match self.eval(a, env)? {
                Value::Null => self.eval(b, env)?,
                v => v,
            },
            Expr::Binary(BinOp::As, a, _) => self.eval(a, env)?,
            Expr::Binary(BinOp::Is, a, t) => {
                let v = self.eval(a, env)?;
                let Expr::Type(t) = &**t else { return err("`is` needs a type") };
                Value::Logical(is_type(&v, *t))
            }
            Expr::Binary(op, a, b) => {
                let a = check(self.eval(a, env)?)?;
                let b = check(self.eval(b, env)?)?;
                binary(*op, &a, &b)?
            }
            Expr::Try(x, otherwise) => {
                let r = self.eval(x, env).and_then(check);
                match (r, otherwise) {
                    (Ok(v), Some(_)) => v,
                    (Err(_), Some(o)) => self.eval(o, env)?,
                    (Ok(v), None) => Value::record(Record::from_pairs(vec![("HasError".into(), Value::Logical(false)), ("Value".into(), v)])),
                    (Err(e), None) => {
                        let msg = e.message();
                        let er = Record::from_pairs(vec![
                            ("Reason".into(), Value::text("Expression.Error")),
                            ("Message".into(), Value::text(&msg)),
                            ("Detail".into(), Value::Null),
                        ]);
                        Value::record(Record::from_pairs(vec![("HasError".into(), Value::Logical(true)), ("Error".into(), Value::record(er))]))
                    }
                }
            }
            Expr::ErrorRaise(x) => {
                let v = self.eval(x, env)?;
                let msg = match &v {
                    Value::Text(t) => t.to_string(),
                    Value::Record(r) => r.get("Message").map(Value::display).unwrap_or_else(|| "Error".into()),
                    other => other.display(),
                };
                return err(msg);
            }
        })
    }

    pub fn lookup(&self, name: &str, env: &Env) -> Result<Value> {
        if let Some(l) = env.find(name) {
            return self.force(&l, name);
        }
        if let Some(v) = self.host.resolve(name) {
            return v;
        }
        if let Some(v) = crate::funcs::constant(name) {
            return Ok(v);
        }
        if crate::funcs::exists(name) {
            return Ok(Value::Function(Rc::new(Function::Builtin(name.to_string()))));
        }
        err(format!("The name '{name}' wasn't recognized. Make sure it's spelled correctly."))
    }

    fn force(&self, l: &Lazy, name: &str) -> Result<Value> {
        let state = std::mem::replace(&mut *l.0.borrow_mut(), LazyState::Busy);
        match state {
            LazyState::Done(r) => {
                *l.0.borrow_mut() = LazyState::Done(r.clone());
                r
            }
            LazyState::Busy => err(format!("A cyclic reference was encountered evaluating '{name}'.")),
            LazyState::Pending(x, weak) => {
                let env = Env(weak.upgrade());
                let r = self.eval(&x, &env);
                *l.0.borrow_mut() = LazyState::Done(r.clone());
                r
            }
        }
    }

    /// Calls a function value.
    pub fn apply(&self, f: &Value, args: Vec<Value>) -> Result<Value> {
        let Value::Function(f) = f else { return err(format!("We cannot convert a value of type {} to type Function.", f.kind())) };
        match &**f {
            Function::Builtin(name) => crate::funcs::call(self, name, args),
            Function::Native(n) => n(self, args),
            Function::Closure { params, body, env } => {
                let mut vars = Vec::with_capacity(params.len());
                let mut it = args.into_iter();
                for p in params {
                    vars.push((p.clone(), it.next().unwrap_or(Value::Null)));
                }
                self.eval(body, &env.with(vars))
            }
        }
    }

    /// Calls a one-argument function on a row (as a record) — `each` bodies see `_` as the row.
    pub fn apply1(&self, f: &Value, arg: Value) -> Result<Value> {
        self.apply(f, vec![arg])
    }
}

fn logical(v: Value) -> Result<Value> {
    match v {
        Value::Logical(_) | Value::Null => Ok(v),
        other => err(format!("We cannot convert the value {} to type Logical.", other.display())),
    }
}

pub fn to_int(v: &Value) -> Result<i64> {
    match v {
        Value::Number(n) if n.is_finite() => Ok(n.round() as i64),
        other => err(format!("We cannot convert the value {} to type Number.", other.display())),
    }
}

pub fn is_type(v: &Value, t: MType) -> bool {
    match t {
        MType::Any => true,
        MType::Null => v.is_null(),
        MType::Number | MType::Currency | MType::Percentage => matches!(v, Value::Number(_)),
        MType::Int64 => matches!(v, Value::Number(n) if n.fract() == 0.0),
        MType::Text => matches!(v, Value::Text(_)),
        MType::Logical => matches!(v, Value::Logical(_)),
        MType::Date => matches!(v, Value::Date(_)),
        MType::DateTime => matches!(v, Value::DateTime(_)),
        MType::Time => matches!(v, Value::Time(_)),
        MType::Duration => matches!(v, Value::Duration(_)),
        MType::Binary => matches!(v, Value::Binary(_)),
        MType::List => matches!(v, Value::List(_)),
        MType::Record => matches!(v, Value::Record(_)),
        MType::Table => matches!(v, Value::Table(_)),
        MType::Function => matches!(v, Value::Function(_)),
        MType::Type => matches!(v, Value::Type(_)),
    }
}

pub fn field(v: &Value, name: &str, opt: bool) -> Result<Value> {
    match v {
        Value::Record(r) => match r.get(name) {
            Some(x) => Ok(x.clone()),
            None if opt => Ok(Value::Null),
            None => err(format!("The field '{name}' of the record wasn't found.")),
        },
        Value::Table(t) => match t.column_index(name) {
            Some(c) => Ok(Value::list(t.rows.iter().map(|r| r.get(c).cloned().unwrap_or_default()).collect())),
            None if opt => Ok(Value::Null),
            None => err(format!("The column '{name}' of the table wasn't found.")),
        },
        Value::Null if opt => Ok(Value::Null),
        other => err(format!("We cannot apply field access to the type {}.", other.kind())),
    }
}

fn project(v: &Value, names: &[String], opt: bool) -> Result<Value> {
    match v {
        Value::Record(r) => {
            let mut out = Record::default();
            for n in names {
                out.names.push(n.clone());
                out.values.push(match r.get(n) {
                    Some(x) => x.clone(),
                    None if opt => Value::Null,
                    None => return err(format!("The field '{n}' of the record wasn't found.")),
                });
            }
            Ok(Value::record(out))
        }
        Value::Table(_) => crate::funcs::table::select_columns(v, names, opt),
        other => err(format!("We cannot apply field access to the type {}.", other.kind())),
    }
}

fn item(v: &Value, i: &Value, opt: bool) -> Result<Value> {
    match (v, i) {
        (Value::List(l), Value::Number(n)) => match l.get(*n as usize).filter(|_| *n >= 0.0) {
            Some(x) => Ok(x.clone()),
            None if opt => Ok(Value::Null),
            None => err("There weren't enough elements in the enumeration to complete the operation."),
        },
        (Value::Table(t), Value::Number(n)) => {
            if *n >= 0.0 && (*n as usize) < t.rows.len() {
                Ok(Value::record(t.row_record(*n as usize)))
            } else if opt {
                Ok(Value::Null)
            } else {
                err("There weren't enough elements in the enumeration to complete the operation.")
            }
        }
        (Value::Table(t), Value::Record(key)) => {
            let cols: Vec<(usize, &Value)> = key.names.iter().zip(key.values.iter()).filter_map(|(n, v)| t.column_index(n).map(|c| (c, v))).collect();
            if cols.len() != key.names.len() {
                return if opt { Ok(Value::Null) } else { err("The key didn't match any rows in the table.") };
            }
            let hits: Vec<usize> = (0..t.rows.len()).filter(|r| cols.iter().all(|(c, v)| values_equal(t.cell(*r, *c), v))).collect();
            match hits.as_slice() {
                [one] => Ok(Value::record(t.row_record(*one))),
                [] if opt => Ok(Value::Null),
                [] => err("The key didn't match any rows in the table."),
                _ => err("The key matched more than one row in the table."),
            }
        }
        (other, _) => err(format!("We cannot apply operator {{}} to a {} value.", other.kind())),
    }
}

fn num_op(op: BinOp, x: f64, y: f64) -> f64 {
    match op {
        BinOp::Add => x + y,
        BinOp::Sub => x - y,
        BinOp::Mul => x * y,
        _ => x / y,
    }
}

pub fn binary(op: BinOp, a: &Value, b: &Value) -> Result<Value> {
    use Value::*;
    let cmp = |o: std::cmp::Ordering| -> Value {
        Logical(match op {
            BinOp::Lt => o.is_lt(),
            BinOp::Le => o.is_le(),
            BinOp::Gt => o.is_gt(),
            _ => o.is_ge(),
        })
    };
    Ok(match op {
        BinOp::Eq => Logical(values_equal(a, b)),
        BinOp::Ne => Logical(!values_equal(a, b)),
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => match (a, b) {
            (Null, _) | (_, Null) => Null,
            (Number(_), Number(_))
            | (Text(_), Text(_))
            | (Logical(_), Logical(_))
            | (Date(_), Date(_))
            | (DateTime(_), DateTime(_))
            | (Date(_), DateTime(_))
            | (DateTime(_), Date(_))
            | (Time(_), Time(_))
            | (Duration(_), Duration(_)) => cmp(compare(a, b)),
            _ => return err(format!("We cannot apply the comparison to types {} and {}.", a.kind(), b.kind())),
        },
        BinOp::Concat => match (a, b) {
            (Null, _) | (_, Null) => Null,
            (Text(x), Text(y)) => Value::text(format!("{x}{y}")),
            (List(x), List(y)) => Value::list(x.iter().chain(y.iter()).cloned().collect()),
            (Record(x), Record(y)) => {
                let mut r = (**x).clone();
                for (n, v) in y.names.iter().zip(y.values.iter()) {
                    match r.names.iter().position(|k| k == n) {
                        Some(i) => {
                            if let Some(slot) = r.values.get_mut(i) {
                                *slot = v.clone();
                            }
                        }
                        None => {
                            r.names.push(n.clone());
                            r.values.push(v.clone());
                        }
                    }
                }
                Value::record(r)
            }
            (Table(_), Table(_)) => crate::funcs::table::combine(&[a.clone(), b.clone()])?,
            (Date(d), Time(t)) => DateTime(*d as f64 + t),
            _ => return err(format!("We cannot apply operator & to types {} and {}.", a.kind(), b.kind())),
        },
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => match (a, b) {
            (Null, _) | (_, Null) => Null,
            (Number(x), Number(y)) => Number(num_op(op, *x, *y)),
            (Date(d), Duration(x)) if matches!(op, BinOp::Add | BinOp::Sub) => {
                let days = if op == BinOp::Add { *d as f64 + x } else { *d as f64 - x };
                Date(days.floor() as i64)
            }
            (Duration(x), Date(d)) if op == BinOp::Add => Date((*d as f64 + x).floor() as i64),
            (DateTime(d), Duration(x)) if matches!(op, BinOp::Add | BinOp::Sub) => DateTime(if op == BinOp::Add { d + x } else { d - x }),
            (Time(d), Duration(x)) if matches!(op, BinOp::Add | BinOp::Sub) => Time((if op == BinOp::Add { d + x } else { d - x }).rem_euclid(1.0)),
            (Date(x), Date(y)) if op == BinOp::Sub => Duration((x - y) as f64),
            (DateTime(x), DateTime(y)) | (Time(x), Time(y)) if op == BinOp::Sub => Duration(x - y),
            (Duration(x), Duration(y)) if matches!(op, BinOp::Add | BinOp::Sub) => Duration(num_op(op, *x, *y)),
            (Duration(x), Number(y)) if matches!(op, BinOp::Mul | BinOp::Div) => Duration(num_op(op, *x, *y)),
            (Number(x), Duration(y)) if op == BinOp::Mul => Duration(x * y),
            _ => return err(format!("We cannot apply the operator to the types {} and {}.", a.kind(), b.kind())),
        },
        BinOp::And | BinOp::Or | BinOp::Coalesce | BinOp::As | BinOp::Is => return err("internal operator error"),
    })
}
