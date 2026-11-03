//! Conversions between M types (`Table.TransformColumnTypes`, `Number.From`, `Text.From`, …),
//! with en-US parsing of numbers, dates and times.

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};

use crate::value::{MType, Value, date_of, days_of, naive_datetime, number_text, serial_of};

pub fn parse_number(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let (neg, t) = match t.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
        Some(inner) => (true, inner.trim()),
        None => (false, t),
    };
    let (pct, t) = match t.strip_suffix('%') {
        Some(x) => (true, x.trim()),
        None => (false, t),
    };
    let t = t.trim_start_matches(['$', '€', '£', '₹', '¥']);
    let cleaned: String = t.chars().filter(|c| *c != ',' && *c != ' ').collect();
    let n = match cleaned.as_str() {
        "NaN" => f64::NAN,
        "Infinity" | "+Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        c => c.parse::<f64>().ok()?,
    };
    let n = if pct { n / 100.0 } else { n };
    Some(if neg { -n } else { n })
}

const DATE_FORMATS: &[&str] = &[
    "%Y-%m-%d",
    "%m/%d/%Y",
    "%Y/%m/%d",
    "%d-%b-%Y",
    "%d %b %Y",
    "%d %B %Y",
    "%b %d, %Y",
    "%B %d, %Y",
    "%b %d %Y",
    "%d.%m.%Y",
    "%Y%m%d",
    "%m-%d-%Y",
    "%d-%m-%Y",
    "%m/%d/%y",
];
const DATETIME_FORMATS: &[&str] = &[
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%dT%H:%M:%S",
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%d %H:%M:%S",
    "%Y-%m-%d %H:%M",
    "%m/%d/%Y %H:%M:%S",
    "%m/%d/%Y %H:%M",
    "%m/%d/%Y %I:%M:%S %p",
    "%m/%d/%Y %I:%M %p",
];

pub fn parse_datetime(s: &str) -> Option<NaiveDateTime> {
    let t = s.trim();
    let t = t.strip_suffix('Z').unwrap_or(t);
    // Drop a time-zone offset (+05:30).
    let t = match t.rfind(['+']) {
        Some(i) if i > 10 => t.get(..i).unwrap_or(t),
        _ => t,
    };
    for f in DATETIME_FORMATS {
        if let Ok(d) = NaiveDateTime::parse_from_str(t, f) {
            return Some(d);
        }
    }
    parse_date(t).and_then(|d| d.and_hms_opt(0, 0, 0))
}

pub fn parse_date(s: &str) -> Option<NaiveDate> {
    let t = s.trim();
    for f in DATE_FORMATS {
        if let Ok(d) = NaiveDate::parse_from_str(t, f) {
            return Some(d);
        }
    }
    for f in DATETIME_FORMATS {
        if let Ok(d) = NaiveDateTime::parse_from_str(t, f) {
            return Some(d.date());
        }
    }
    None
}

pub fn parse_time(s: &str) -> Option<NaiveTime> {
    let t = s.trim();
    for f in ["%H:%M:%S%.f", "%H:%M:%S", "%H:%M", "%I:%M:%S %p", "%I:%M %p", "%I %p"] {
        if let Ok(x) = NaiveTime::parse_from_str(t, f) {
            return Some(x);
        }
    }
    parse_datetime(t).map(|d| d.time())
}

fn time_serial(t: NaiveTime) -> f64 {
    t.num_seconds_from_midnight() as f64 / 86400.0 + t.nanosecond() as f64 / 86_400_000_000_000.0
}

/// `Text.From`: en-US text of a value; null stays null.
pub fn text_from(v: &Value) -> Value {
    match v {
        Value::Null => Value::Null,
        Value::Text(_) => v.clone(),
        Value::Number(n) => Value::text(number_text(*n)),
        Value::Logical(b) => Value::text(if *b { "true" } else { "false" }),
        Value::Date(d) => Value::text(date_of(*d).map(|d| format!("{}/{}/{}", d.month(), d.day(), d.year())).unwrap_or_default()),
        Value::DateTime(x) => Value::text(naive_datetime(*x).map(|d| d.format("%-m/%-d/%Y %-I:%M:%S %p").to_string()).unwrap_or_default()),
        Value::Time(x) => Value::text(crate::value::time_text(*x)),
        Value::Duration(x) => Value::text(crate::value::duration_text(*x)),
        Value::Error(_) => v.clone(),
        other => Value::error(format!("We cannot convert a value of type {} to type Text.", other.kind())),
    }
}

/// The text a value stands for when text functions are applied to it (`Text.Upper(5)` is an
/// error in M; transforms on mixed columns convert first, as the UI does).
pub fn as_text(v: &Value) -> Option<String> {
    match text_from(v) {
        Value::Text(t) => Some(t.to_string()),
        _ => None,
    }
}

fn conv_error(v: &Value, to: &str) -> Value {
    Value::error(format!("DataFormat.Error: We couldn't convert to {to}. Details: {}", v.display()))
}

/// Converts a value to a type; a value that can't be converted becomes an error value.
pub fn to_type(v: &Value, t: MType) -> Value {
    if v.is_null() || v.is_error() {
        return v.clone();
    }
    // Blank text (an empty CSV field) becomes null in every type but text.
    if matches!(v, Value::Text(s) if s.trim().is_empty()) && !matches!(t, MType::Text | MType::Any) {
        return Value::Null;
    }
    match t {
        MType::Any | MType::Null => v.clone(),
        MType::Number | MType::Currency | MType::Percentage => match number_from(v) {
            Some(n) if t == MType::Currency => Value::Number((n * 10_000.0).round() / 10_000.0),
            Some(n) => Value::Number(n),
            None => conv_error(v, "Number"),
        },
        MType::Int64 => match number_from(v) {
            Some(n) => Value::Number(n.round_ties_even()),
            None => conv_error(v, "Number"),
        },
        MType::Text => text_from(v),
        MType::Logical => match v {
            Value::Logical(_) => v.clone(),
            Value::Number(n) => Value::Logical(*n != 0.0),
            Value::Text(s) => match s.trim().to_ascii_lowercase().as_str() {
                "true" => Value::Logical(true),
                "false" => Value::Logical(false),
                _ => conv_error(v, "Logical"),
            },
            _ => conv_error(v, "Logical"),
        },
        MType::Date => match v {
            Value::Date(_) => v.clone(),
            Value::DateTime(x) => Value::Date(x.floor() as i64),
            Value::Number(n) => Value::Date(n.floor() as i64),
            Value::Text(s) => parse_date(s).map(|d| Value::Date(days_of(d))).unwrap_or_else(|| conv_error(v, "Date")),
            _ => conv_error(v, "Date"),
        },
        MType::DateTime => match v {
            Value::DateTime(_) => v.clone(),
            Value::Date(d) => Value::DateTime(*d as f64),
            Value::Number(n) => Value::DateTime(*n),
            Value::Text(s) => parse_datetime(s).map(|d| Value::DateTime(serial_of(d))).unwrap_or_else(|| conv_error(v, "DateTime")),
            _ => conv_error(v, "DateTime"),
        },
        MType::Time => match v {
            Value::Time(_) => v.clone(),
            Value::DateTime(x) => Value::Time(x.rem_euclid(1.0)),
            Value::Number(n) => Value::Time(n.rem_euclid(1.0)),
            Value::Text(s) => parse_time(s).map(|x| Value::Time(time_serial(x))).unwrap_or_else(|| conv_error(v, "Time")),
            _ => conv_error(v, "Time"),
        },
        MType::Duration => match v {
            Value::Duration(_) => v.clone(),
            Value::Number(n) => Value::Duration(*n),
            Value::Text(s) => parse_duration(s).map(Value::Duration).unwrap_or_else(|| conv_error(v, "Duration")),
            _ => conv_error(v, "Duration"),
        },
        MType::Binary => match v {
            Value::Binary(_) => v.clone(),
            Value::Text(s) => Value::Binary(std::rc::Rc::new(s.as_bytes().to_vec())),
            _ => conv_error(v, "Binary"),
        },
        MType::List | MType::Record | MType::Table | MType::Function | MType::Type => {
            if crate::m::eval::is_type(v, t) {
                v.clone()
            } else {
                conv_error(v, t.name())
            }
        }
    }
}

pub fn number_from(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::Logical(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::Text(s) => parse_number(s),
        Value::Date(d) => Some(*d as f64),
        Value::DateTime(x) | Value::Time(x) | Value::Duration(x) => Some(*x),
        _ => None,
    }
}

/// `d.hh:mm:ss` or `hh:mm:ss`.
pub fn parse_duration(s: &str) -> Option<f64> {
    let t = s.trim();
    let (neg, t) = t.strip_prefix('-').map(|x| (true, x)).unwrap_or((false, t));
    let (days, rest) = match t.split_once('.') {
        Some((d, r)) if r.contains(':') => (d.parse::<f64>().ok()?, r),
        _ => (0.0, t),
    };
    let parts: Vec<f64> = rest.split(':').map(|p| p.parse::<f64>()).collect::<std::result::Result<_, _>>().ok()?;
    let secs = match parts.as_slice() {
        [h, m, s] => h * 3600.0 + m * 60.0 + s,
        [h, m] => h * 3600.0 + m * 60.0,
        [d] => return Some(if neg { -d } else { *d }),
        _ => return None,
    };
    let x = days + secs / 86400.0;
    Some(if neg { -x } else { x })
}

/// The type a column's values suggest (Detect Data Type): whole numbers, decimals, dates, date
/// times, logicals or text. Text that reads as numbers or dates suggests those types.
pub fn detect(values: &[Value]) -> MType {
    let mut kinds = std::collections::HashSet::new();
    for v in values.iter().filter(|v| !v.is_empty_cell()).take(1000) {
        let k = match v {
            Value::Number(n) => {
                if n.fract() == 0.0 {
                    MType::Int64
                } else {
                    MType::Number
                }
            }
            Value::Logical(_) => MType::Logical,
            Value::Date(_) => MType::Date,
            Value::DateTime(_) => MType::DateTime,
            Value::Time(_) => MType::Time,
            Value::Duration(_) => MType::Duration,
            Value::Text(s) => {
                let s = s.trim();
                if let Some(n) = parse_number(s).filter(|_| !s.contains('/') && !s.contains(':') && s.chars().filter(|c| *c == '-').count() <= 1) {
                    if n.fract() == 0.0 { MType::Int64 } else { MType::Number }
                } else if matches!(s.to_ascii_lowercase().as_str(), "true" | "false") {
                    MType::Logical
                } else if parse_date(s).is_some() && s.len() <= 12 {
                    MType::Date
                } else if parse_datetime(s).is_some() {
                    MType::DateTime
                } else {
                    MType::Text
                }
            }
            _ => MType::Any,
        };
        kinds.insert(k);
    }
    match kinds.len() {
        0 => MType::Any,
        1 => kinds.into_iter().next().unwrap_or(MType::Any),
        _ if kinds.iter().all(|k| matches!(k, MType::Int64 | MType::Number)) => MType::Number,
        _ if kinds.iter().all(|k| matches!(k, MType::Date | MType::DateTime)) => MType::DateTime,
        _ => MType::Text,
    }
}

pub fn date_parts(v: &Value) -> Option<NaiveDateTime> {
    match v {
        Value::Date(d) => date_of(*d).and_then(|d| d.and_hms_opt(0, 0, 0)),
        Value::DateTime(x) => naive_datetime(*x),
        Value::Text(s) => parse_datetime(s),
        _ => None,
    }
}

pub fn year_of(v: &Value) -> Option<i32> {
    date_parts(v).map(|d| d.year())
}
