//! A serializable scalar: what filters, parameters, bookmarks and query results hold.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use vizcraft_datamodel::Value as DmValue;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    #[default]
    Null,
    Bool(bool),
    Num(f64),
    Text(String),
}

impl Scalar {
    pub fn from_dm(v: &DmValue) -> Scalar {
        match v {
            DmValue::Blank => Scalar::Null,
            DmValue::Number(n) => Scalar::Num(*n),
            DmValue::Text(t) => Scalar::Text(t.to_string()),
            DmValue::Bool(b) => Scalar::Bool(*b),
        }
    }
    pub fn to_dm(&self) -> DmValue {
        match self {
            Scalar::Null => DmValue::Blank,
            Scalar::Num(n) => DmValue::Number(*n),
            Scalar::Text(t) => DmValue::text(t),
            Scalar::Bool(b) => DmValue::Bool(*b),
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Scalar::Num(n) => Some(*n),
            Scalar::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Scalar::Text(t) => t.trim().parse().ok(),
            Scalar::Null => None,
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Scalar::Null)
    }
    /// Plain display text (numbers without formatting).
    pub fn text(&self) -> String {
        match self {
            Scalar::Null => "(Blank)".into(),
            Scalar::Bool(b) => if *b { "True".into() } else { "False".into() },
            Scalar::Num(n) => crate::numfmt::general(*n),
            Scalar::Text(t) => t.clone(),
        }
    }
    /// A DAX literal for this value.
    pub fn dax(&self) -> String {
        match self {
            Scalar::Null => "BLANK()".into(),
            Scalar::Bool(b) => if *b { "TRUE()".into() } else { "FALSE()".into() },
            Scalar::Num(n) => {
                if n.is_finite() {
                    format!("{n}")
                } else {
                    "BLANK()".into()
                }
            }
            Scalar::Text(t) => format!("\"{}\"", t.replace('"', "\"\"")),
        }
    }
    /// A stable key used for selections and highlight matching.
    pub fn key(&self) -> String {
        match self {
            Scalar::Null => "\u{0}null".into(),
            Scalar::Bool(b) => format!("\u{0}b{b}"),
            Scalar::Num(n) => format!("{n}"),
            Scalar::Text(t) => t.to_lowercase(),
        }
    }
}

pub fn compare(a: &Scalar, b: &Scalar) -> Ordering {
    match (a, b) {
        (Scalar::Num(x), Scalar::Num(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Scalar::Text(x), Scalar::Text(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
        (Scalar::Null, Scalar::Null) => Ordering::Equal,
        (Scalar::Null, _) => Ordering::Less,
        (_, Scalar::Null) => Ordering::Greater,
        (Scalar::Num(_), _) => Ordering::Less,
        (_, Scalar::Num(_)) => Ordering::Greater,
        (Scalar::Bool(x), Scalar::Bool(y)) => x.cmp(y),
        (Scalar::Bool(_), _) => Ordering::Less,
        (_, Scalar::Bool(_)) => Ordering::Greater,
    }
}
