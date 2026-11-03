//! M values: scalars, lists, records, tables, functions and types.

use std::fmt::Write as _;
use std::rc::Rc;

use chrono::{Datelike, NaiveDate, Timelike};

use crate::m::eval::Function;

/// The types M knows, as the column type of a table and as a value (`type text`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MType {
    #[default]
    Any,
    Null,
    Number,
    Int64,
    Currency,
    Percentage,
    Text,
    Logical,
    Date,
    DateTime,
    Time,
    Duration,
    Binary,
    List,
    Record,
    Table,
    Function,
    Type,
}

impl MType {
    /// The M spelling (`type text`, `Int64.Type`).
    pub fn m(self) -> &'static str {
        match self {
            MType::Any => "type any",
            MType::Null => "type null",
            MType::Number => "type number",
            MType::Int64 => "Int64.Type",
            MType::Currency => "Currency.Type",
            MType::Percentage => "Percentage.Type",
            MType::Text => "type text",
            MType::Logical => "type logical",
            MType::Date => "type date",
            MType::DateTime => "type datetime",
            MType::Time => "type time",
            MType::Duration => "type duration",
            MType::Binary => "type binary",
            MType::List => "type list",
            MType::Record => "type record",
            MType::Table => "type table",
            MType::Function => "type function",
            MType::Type => "type type",
        }
    }

    /// A short label for column headers.
    pub fn label(self) -> &'static str {
        match self {
            MType::Any => "ABC/123",
            MType::Null => "null",
            MType::Number => "1.2",
            MType::Int64 => "123",
            MType::Currency => "$",
            MType::Percentage => "%",
            MType::Text => "ABC",
            MType::Logical => "T/F",
            MType::Date => "Date",
            MType::DateTime => "DateTime",
            MType::Time => "Time",
            MType::Duration => "Duration",
            MType::Binary => "Binary",
            MType::List => "List",
            MType::Record => "Record",
            MType::Table => "Table",
            MType::Function => "fx",
            MType::Type => "Type",
        }
    }

    /// Names for menus.
    pub fn name(self) -> &'static str {
        match self {
            MType::Any => "Any",
            MType::Null => "Null",
            MType::Number => "Decimal Number",
            MType::Int64 => "Whole Number",
            MType::Currency => "Fixed Decimal Number",
            MType::Percentage => "Percentage",
            MType::Text => "Text",
            MType::Logical => "True/False",
            MType::Date => "Date",
            MType::DateTime => "Date/Time",
            MType::Time => "Time",
            MType::Duration => "Duration",
            MType::Binary => "Binary",
            MType::List => "List",
            MType::Record => "Record",
            MType::Table => "Table",
            MType::Function => "Function",
            MType::Type => "Type",
        }
    }

    /// The types a column can be changed to.
    pub const COLUMN_TYPES: [MType; 11] = [
        MType::Number,
        MType::Currency,
        MType::Int64,
        MType::Percentage,
        MType::DateTime,
        MType::Date,
        MType::Time,
        MType::Duration,
        MType::Text,
        MType::Logical,
        MType::Any,
    ];

    pub fn is_numeric(self) -> bool {
        matches!(self, MType::Number | MType::Int64 | MType::Currency | MType::Percentage)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    pub names: Vec<String>,
    pub values: Vec<Value>,
}

impl Record {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.names.iter().position(|n| n == name).and_then(|i| self.values.get(i))
    }

    pub fn from_pairs(pairs: Vec<(String, Value)>) -> Record {
        let (names, values) = pairs.into_iter().unzip();
        Record { names, values }
    }
}

/// A table: column names, column types and rows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub columns: Vec<String>,
    pub types: Vec<MType>,
    pub rows: Vec<Vec<Value>>,
}

impl Table {
    pub fn new(columns: Vec<String>, rows: Vec<Vec<Value>>) -> Table {
        let types = vec![MType::Any; columns.len()];
        Table { columns, types, rows }
    }

    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }

    pub fn cell(&self, row: usize, col: usize) -> &Value {
        self.rows.get(row).and_then(|r| r.get(col)).unwrap_or_else(|| null_ref())
    }

    pub fn row_record(&self, row: usize) -> Record {
        Record { names: self.columns.clone(), values: (0..self.columns.len()).map(|c| self.cell(row, c).clone()).collect() }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

thread_local! {
    static NULL: &'static Value = Box::leak(Box::new(Value::Null));
}

/// A `null` to point at where a reference is needed (missing cells and arguments).
pub fn null_ref() -> &'static Value {
    NULL.with(|n| *n)
}

/// An M value. Errors are values too: a cell can hold one (shown as "Error" in the preview).
#[derive(Clone, Debug, Default)]
pub enum Value {
    #[default]
    Null,
    Logical(bool),
    Number(f64),
    Text(Rc<str>),
    /// Days since 1899-12-30.
    Date(i64),
    /// Days (with fraction) since 1899-12-30.
    DateTime(f64),
    /// Fraction of a day.
    Time(f64),
    /// Days.
    Duration(f64),
    Binary(Rc<Vec<u8>>),
    List(Rc<Vec<Value>>),
    Record(Rc<Record>),
    Table(Rc<Table>),
    Function(Rc<Function>),
    Type(MType),
    Error(Rc<str>),
}

impl PartialEq for Value {
    fn eq(&self, other: &Value) -> bool {
        values_equal(self, other)
    }
}

pub fn values_equal(a: &Value, b: &Value) -> bool {
    use Value::*;
    match (a, b) {
        (Null, Null) => true,
        (Logical(x), Logical(y)) => x == y,
        (Number(x), Number(y)) => x == y,
        (Text(x), Text(y)) => x == y,
        (Date(x), Date(y)) => x == y,
        (DateTime(x), DateTime(y)) | (Time(x), Time(y)) | (Duration(x), Duration(y)) => x == y,
        (Binary(x), Binary(y)) => x == y,
        (List(x), List(y)) => x.len() == y.len() && x.iter().zip(y.iter()).all(|(a, b)| values_equal(a, b)),
        (Record(x), Record(y)) => x == y,
        (Table(x), Table(y)) => x == y,
        (Type(x), Type(y)) => x == y,
        (Function(x), Function(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

pub fn epoch() -> NaiveDate {
    NaiveDate::from_ymd_opt(1899, 12, 30).unwrap_or_default()
}

pub fn date_of(days: i64) -> Option<NaiveDate> {
    epoch().checked_add_signed(chrono::Duration::days(days))
}

pub fn days_of(d: NaiveDate) -> i64 {
    d.signed_duration_since(epoch()).num_days()
}

impl Value {
    pub fn text(s: impl AsRef<str>) -> Value {
        Value::Text(Rc::from(s.as_ref()))
    }

    pub fn error(msg: impl AsRef<str>) -> Value {
        Value::Error(Rc::from(msg.as_ref()))
    }

    pub fn list(v: Vec<Value>) -> Value {
        Value::List(Rc::new(v))
    }

    pub fn table(t: Table) -> Value {
        Value::Table(Rc::new(t))
    }

    pub fn record(r: Record) -> Value {
        Value::Record(Rc::new(r))
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Value::Error(_))
    }

    /// Null or empty text: what profiling counts as empty.
    pub fn is_empty_cell(&self) -> bool {
        match self {
            Value::Null => true,
            Value::Text(t) => t.is_empty(),
            _ => false,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(t) => Some(t),
            _ => None,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Logical(_) => "logical",
            Value::Number(_) => "number",
            Value::Text(_) => "text",
            Value::Date(_) => "date",
            Value::DateTime(_) => "datetime",
            Value::Time(_) => "time",
            Value::Duration(_) => "duration",
            Value::Binary(_) => "binary",
            Value::List(_) => "list",
            Value::Record(_) => "record",
            Value::Table(_) => "table",
            Value::Function(_) => "function",
            Value::Type(_) => "type",
            Value::Error(_) => "error",
        }
    }

    /// The text a cell shows in the preview grid.
    pub fn display(&self) -> String {
        match self {
            Value::Null => "null".into(),
            Value::Logical(b) => if *b { "TRUE" } else { "FALSE" }.into(),
            Value::Number(n) => number_text(*n),
            Value::Text(t) => t.to_string(),
            Value::Date(d) => date_of(*d).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_else(|| d.to_string()),
            Value::DateTime(x) => datetime_text(*x),
            Value::Time(x) => time_text(*x),
            Value::Duration(x) => duration_text(*x),
            Value::Binary(b) => format!("Binary ({} bytes)", b.len()),
            Value::List(_) => "List".into(),
            Value::Record(_) => "Record".into(),
            Value::Table(_) => "Table".into(),
            Value::Function(_) => "Function".into(),
            Value::Type(t) => t.m().trim_start_matches("type ").into(),
            Value::Error(_) => "Error".into(),
        }
    }

    /// The M literal for a scalar (`"text"`, `12`, `#date(2024, 1, 31)`, `null`).
    pub fn m_literal(&self) -> String {
        match self {
            Value::Null => "null".into(),
            Value::Logical(b) => if *b { "true" } else { "false" }.into(),
            Value::Number(n) => number_text(*n),
            Value::Text(t) => quote_text(t),
            Value::Date(d) => date_of(*d).map(|d| format!("#date({}, {}, {})", d.year(), d.month(), d.day())).unwrap_or_else(|| "null".into()),
            Value::DateTime(x) => {
                let days = x.floor();
                let secs = ((x - days) * 86400.0).round() as u32;
                date_of(days as i64)
                    .map(|d| format!("#datetime({}, {}, {}, {}, {}, {})", d.year(), d.month(), d.day(), secs / 3600, secs / 60 % 60, secs % 60))
                    .unwrap_or_else(|| "null".into())
            }
            Value::Time(x) => {
                let secs = (x * 86400.0).round() as u32;
                format!("#time({}, {}, {})", secs / 3600 % 24, secs / 60 % 60, secs % 60)
            }
            Value::Duration(x) => {
                let d = x.trunc();
                let secs = ((x - d) * 86400.0).round() as i64;
                format!("#duration({}, {}, {}, {})", d as i64, secs / 3600, secs / 60 % 60, secs % 60)
            }
            Value::List(l) => format!("{{{}}}", l.iter().map(Value::m_literal).collect::<Vec<_>>().join(", ")),
            _ => "null".into(),
        }
    }
}

pub fn quote_text(t: &str) -> String {
    let mut s = String::with_capacity(t.len() + 2);
    s.push('"');
    for c in t.chars() {
        match c {
            '"' => s.push_str("\"\""),
            '\n' => s.push_str("#(lf)"),
            '\r' => s.push_str("#(cr)"),
            '\t' => s.push_str("#(tab)"),
            c => s.push(c),
        }
    }
    s.push('"');
    s
}

pub fn number_text(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if n.fract() == 0.0 && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    // At most 15 significant digits, as the editor and Excel show numbers.
    let mag = n.abs().log10().floor() as i32;
    let n = if (-300..300).contains(&mag) {
        let f = 10f64.powi(14 - mag);
        if f.is_finite() && f > 0.0 { (n * f).round() / f } else { n }
    } else {
        n
    };
    let mut s = String::new();
    let _ = write!(s, "{n}");
    s
}

pub fn datetime_text(x: f64) -> String {
    let days = x.floor();
    let secs = ((x - days) * 86400.0).round() as u32;
    match date_of(days as i64).and_then(|d| d.and_hms_opt(secs / 3600 % 24, secs / 60 % 60, secs % 60)) {
        Some(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => number_text(x),
    }
}

pub fn time_text(x: f64) -> String {
    let secs = (x.rem_euclid(1.0) * 86400.0).round() as u32;
    format!("{:02}:{:02}:{:02}", secs / 3600 % 24, secs / 60 % 60, secs % 60)
}

pub fn duration_text(x: f64) -> String {
    let neg = x < 0.0;
    let x = x.abs();
    let d = x.trunc() as i64;
    let secs = ((x - x.trunc()) * 86400.0).round() as i64;
    format!("{}{}.{:02}:{:02}:{:02}", if neg { "-" } else { "" }, d, secs / 3600, secs / 60 % 60, secs % 60)
}

/// The datetime parts of a serial, for Date/DateTime functions.
pub fn naive_datetime(x: f64) -> Option<chrono::NaiveDateTime> {
    let days = x.floor();
    let secs = ((x - days) * 86400.0).round() as u32;
    date_of(days as i64)?.and_hms_opt(secs / 3600 % 24, secs / 60 % 60, secs % 60)
}

pub fn serial_of(dt: chrono::NaiveDateTime) -> f64 {
    days_of(dt.date()) as f64 + dt.time().num_seconds_from_midnight() as f64 / 86400.0
}

/// Orders values for sorting: nulls first, then by kind, then by value (text by code point, as M
/// does by default).
pub fn compare(a: &Value, b: &Value) -> std::cmp::Ordering {
    use Value::*;
    use std::cmp::Ordering;
    fn rank(v: &Value) -> u8 {
        match v {
            Null => 0,
            Logical(_) => 1,
            Number(_) => 2,
            Date(_) | DateTime(_) => 3,
            Time(_) => 4,
            Duration(_) => 5,
            Text(_) => 6,
            Error(_) => 9,
            _ => 8,
        }
    }
    match (a, b) {
        (Number(x), Number(y)) => x.total_cmp(y),
        (Text(x), Text(y)) => x.cmp(y),
        (Logical(x), Logical(y)) => x.cmp(y),
        (Date(x), Date(y)) => x.cmp(y),
        (Date(x), DateTime(y)) => (*x as f64).total_cmp(y),
        (DateTime(x), Date(y)) => x.total_cmp(&(*y as f64)),
        (DateTime(x), DateTime(y)) | (Time(x), Time(y)) | (Duration(x), Duration(y)) => x.total_cmp(y),
        _ => rank(a).cmp(&rank(b)).then(Ordering::Equal),
    }
}
