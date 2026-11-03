//! Scalar values of the data model and their DAX semantics (blank coercion, case-insensitive text).

use std::cmp::Ordering;
use std::sync::Arc;

use crate::{Error, Result};

/// One scalar in a data-model column or a DAX result.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    #[default]
    Blank,
    Number(f64),
    Text(Arc<str>),
    Bool(bool),
}

/// The dictionary key of a value: numbers by bits (with `-0` folded into `0`), text without case,
/// as DAX compares text case-insensitively.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    Blank,
    Bool(bool),
    Number(u64),
    Text(String),
}

impl Value {
    pub fn text(s: impl AsRef<str>) -> Value {
        Value::Text(Arc::from(s.as_ref()))
    }

    pub fn is_blank(&self) -> bool {
        matches!(self, Value::Blank)
    }

    pub fn key(&self) -> Key {
        match self {
            Value::Blank => Key::Blank,
            Value::Bool(b) => Key::Bool(*b),
            Value::Number(n) => Key::Number(if *n == 0.0 { 0.0f64.to_bits() } else { n.to_bits() }),
            Value::Text(t) => Key::Text(t.to_lowercase()),
        }
    }

    /// The number a value stands for in arithmetic: blank is 0, booleans are 1/0, numeric text is
    /// converted, other text is an error.
    pub fn to_number(&self) -> Result<f64> {
        match self {
            Value::Blank => Ok(0.0),
            Value::Number(n) => Ok(*n),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Value::Text(t) => t.trim().parse::<f64>().map_err(|_| Error::Eval(format!("cannot convert the text \"{t}\" to a number"))),
        }
    }

    /// The number a value holds when it is numeric (aggregations skip everything else).
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// DAX truthiness: blank is false, numbers are true when non-zero.
    pub fn truthy(&self) -> Result<bool> {
        match self {
            Value::Bool(b) => Ok(*b),
            Value::Blank => Ok(false),
            Value::Number(n) => Ok(*n != 0.0),
            Value::Text(t) => match t.to_ascii_lowercase().as_str() {
                "true" => Ok(true),
                "false" | "" => Ok(false),
                _ => Err(Error::Eval(format!("cannot convert the text \"{t}\" to True/False"))),
            },
        }
    }

    pub fn to_text(&self) -> String {
        match self {
            Value::Blank => String::new(),
            Value::Number(n) => format_number(*n),
            Value::Text(t) => t.to_string(),
            Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Value::Blank => serde_json::Value::Null,
            Value::Number(n) => serde_json::Number::from_f64(*n).map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null),
            Value::Text(t) => serde_json::Value::String(t.to_string()),
            Value::Bool(b) => serde_json::Value::Bool(*b),
        }
    }
}

fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") }
}

/// Orders two values the way DAX comparison operators do: blank takes the other side's empty value
/// (0, "" or FALSE), numbers sort before text, text compares without case.
pub fn compare(a: &Value, b: &Value) -> Ordering {
    use Value::{Blank, Bool, Number, Text};
    match (a, b) {
        (Blank, Blank) => Ordering::Equal,
        (Blank, Number(n)) => 0.0f64.total_cmp(n),
        (Number(n), Blank) => n.total_cmp(&0.0),
        (Blank, Text(t)) => {
            if t.is_empty() {
                Ordering::Equal
            } else {
                Ordering::Less
            }
        }
        (Text(t), Blank) => {
            if t.is_empty() {
                Ordering::Equal
            } else {
                Ordering::Greater
            }
        }
        (Blank, Bool(b)) => false.cmp(b),
        (Bool(b), Blank) => b.cmp(&false),
        (Number(x), Number(y)) => x.total_cmp(y),
        (Text(x), Text(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
        (Bool(x), Bool(y)) => x.cmp(y),
        (Number(x), Bool(y)) => x.total_cmp(&if *y { 1.0 } else { 0.0 }),
        (Bool(x), Number(y)) => (if *x { 1.0f64 } else { 0.0 }).total_cmp(y),
        (Number(_) | Bool(_), Text(_)) => Ordering::Less,
        (Text(_), Number(_) | Bool(_)) => Ordering::Greater,
    }
}

/// Sort order for result rows: blanks first, then numbers, booleans and text.
pub fn sort_order(a: &Value, b: &Value) -> Ordering {
    fn rank(v: &Value) -> u8 {
        match v {
            Value::Blank => 0,
            Value::Number(_) => 1,
            Value::Bool(_) => 2,
            Value::Text(_) => 3,
        }
    }
    rank(a).cmp(&rank(b)).then_with(|| compare(a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_equals_zero_and_empty_text() {
        assert_eq!(compare(&Value::Blank, &Value::Number(0.0)), Ordering::Equal);
        assert_eq!(compare(&Value::Blank, &Value::text("")), Ordering::Equal);
        assert_eq!(compare(&Value::text("East"), &Value::text("east")), Ordering::Equal);
        assert_eq!(Value::text("East").key(), Value::text("EAST").key());
        assert_eq!(Value::Number(-0.0).key(), Value::Number(0.0).key());
    }
}
