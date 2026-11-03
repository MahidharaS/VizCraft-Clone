//! `Record.*`, `Value.*`, `Logical.*`, `Error.*`, comparers, splitters, combiners, replacers.

use super::convert::text_from;
use super::{arg, err, list_arg, names_arg, rec, text_arg};
use crate::Result;
use crate::m::eval::{Evaluator, field};
use crate::value::{MType, Record, Table, Value, compare, values_equal};

pub fn call(_ev: &Evaluator, name: &str, a: &[Value]) -> Option<Result<Value>> {
    if let Some(r) = super::table::splitter(name, a) {
        return Some(r);
    }
    if let Some(r) = super::table::replacer(name, a) {
        return Some(r);
    }
    let record = || match arg(a, 0) {
        Value::Record(r) => Ok(r.clone()),
        other => err(format!("expected a record, got {}", other.kind())),
    };
    let r = match name {
        "Record.Field" => text_arg(a, 1, name).and_then(|n| field(arg(a, 0), &n, false)),
        "Record.FieldOrDefault" => {
            text_arg(a, 1, name).and_then(|n| Ok(field(arg(a, 0), &n, true)?).map(|v| if v.is_null() { arg(a, 2).clone() } else { v }))
        }
        "Record.FieldNames" => record().map(|r| Value::list(r.names.iter().map(Value::text).collect())),
        "Record.FieldValues" | "Record.ToList" => record().map(|r| Value::list(r.values.clone())),
        "Record.FieldCount" => record().map(|r| Value::Number(r.names.len() as f64)),
        "Record.HasFields" => record().and_then(|r| Ok(Value::Logical(names_arg(arg(a, 1))?.iter().all(|n| r.get(n).is_some())))),
        "Record.AddField" => record().and_then(|r| {
            let mut r = (*r).clone();
            r.names.push(text_arg(a, 1, name)?);
            r.values.push(arg(a, 2).clone());
            Ok(Value::record(r))
        }),
        "Record.RemoveFields" | "Record.SelectFields" => record().and_then(|r| {
            let names = names_arg(arg(a, 1))?;
            let keep = name == "Record.SelectFields";
            let pairs = r.names.iter().zip(r.values.iter()).filter(|(n, _)| names.contains(n) == keep).map(|(n, v)| (n.clone(), v.clone())).collect();
            Ok(Value::record(Record::from_pairs(pairs)))
        }),
        "Record.RenameFields" => record().map(|r| {
            let mut r = (*r).clone();
            if let Value::List(l) = arg(a, 1) {
                let pairs: Vec<Vec<Value>> = if l.first().is_some_and(|x| matches!(x, Value::List(_))) {
                    l.iter().filter_map(|x| if let Value::List(p) = x { Some(p.to_vec()) } else { None }).collect()
                } else {
                    vec![l.to_vec()]
                };
                for p in pairs {
                    if let (Some(Value::Text(o)), Some(Value::Text(n))) = (p.first(), p.get(1))
                        && let Some(i) = r.names.iter().position(|x| **x == **o)
                        && let Some(slot) = r.names.get_mut(i)
                    {
                        *slot = n.to_string();
                    }
                }
            }
            Value::record(r)
        }),
        "Record.ToTable" => record().map(|r| {
            Value::table(Table::new(
                vec!["Name".into(), "Value".into()],
                r.names.iter().zip(r.values.iter()).map(|(n, v)| vec![Value::text(n), v.clone()]).collect(),
            ))
        }),
        "Record.FromList" => list_arg(a, 0, name).and_then(|l| {
            let names = names_arg(arg(a, 1))?;
            Ok(Value::record(Record { names, values: l.to_vec() }))
        }),
        "Value.Equals" => Ok(Value::Logical(values_equal(arg(a, 0), arg(a, 1)))),
        "Value.Compare" => Ok(Value::Number(match compare(arg(a, 0), arg(a, 1)) {
            std::cmp::Ordering::Less => -1.0,
            std::cmp::Ordering::Equal => 0.0,
            std::cmp::Ordering::Greater => 1.0,
        })),
        "Value.Is" => Ok(Value::Logical(match arg(a, 1) {
            Value::Type(t) => crate::m::eval::is_type(arg(a, 0), *t),
            _ => false,
        })),
        "Value.Type" => Ok(Value::Type(match arg(a, 0) {
            Value::Null => MType::Null,
            Value::Logical(_) => MType::Logical,
            Value::Number(_) => MType::Number,
            Value::Text(_) => MType::Text,
            Value::Date(_) => MType::Date,
            Value::DateTime(_) => MType::DateTime,
            Value::Time(_) => MType::Time,
            Value::Duration(_) => MType::Duration,
            Value::Binary(_) => MType::Binary,
            Value::List(_) => MType::List,
            Value::Record(_) => MType::Record,
            Value::Table(_) => MType::Table,
            Value::Function(_) => MType::Function,
            Value::Type(_) => MType::Type,
            Value::Error(_) => MType::Any,
        })),
        "Value.ReplaceType" => Ok(arg(a, 0).clone()),
        "Logical.From" => Ok(super::convert::to_type(arg(a, 0), MType::Logical)),
        "Logical.FromText" => Ok(match arg(a, 0).as_text().map(str::to_ascii_lowercase).as_deref() {
            Some("true") => Value::Logical(true),
            Some("false") => Value::Logical(false),
            _ => Value::error("DataFormat.Error: We couldn't convert to Logical."),
        }),
        "Logical.ToText" => Ok(text_from(arg(a, 0))),
        "Error.Record" => Ok(rec(vec![("Reason", arg(a, 0).clone()), ("Message", arg(a, 1).clone()), ("Detail", arg(a, 2).clone())])),
        "Comparer.Ordinal" | "Comparer.OrdinalIgnoreCase" => Ok(Value::Number(match (arg(a, 0).as_text(), arg(a, 1).as_text()) {
            (Some(x), Some(y)) => {
                let o = if name == "Comparer.Ordinal" { x.cmp(y) } else { x.to_lowercase().cmp(&y.to_lowercase()) };
                o as i8 as f64
            }
            _ => 0.0,
        })),
        _ => return None,
    };
    Some(r)
}
