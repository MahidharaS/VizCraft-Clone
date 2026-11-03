//! `Text.*` and `Character.*` functions.

use super::convert::{as_text, text_from};
use super::{arg, count_arg, err, list_arg, opt_num, text_arg};
use crate::Result;
use crate::m::eval::Evaluator;
use crate::value::Value;

fn chars(s: &str) -> Vec<char> {
    s.chars().collect()
}

fn slice(c: &[char], from: usize, len: usize) -> String {
    let from = from.min(c.len());
    let to = from.saturating_add(len).min(c.len());
    c.get(from..to).unwrap_or_default().iter().collect()
}

/// Text functions return null for a null text argument, as M does.
fn text_or_null(a: &[Value], f: &str) -> Result<Option<String>> {
    match arg(a, 0) {
        Value::Null => Ok(None),
        Value::Text(t) => Ok(Some(t.to_string())),
        Value::Error(e) => err(e.to_string()),
        other => err(format!("{f}: we cannot convert the value {} to type Text.", other.display())),
    }
}

/// Finds the start of the `index`-th occurrence of `d` (from the end when `from_end`).
fn nth_delim(s: &str, d: &str, index: usize, from_end: bool) -> Option<usize> {
    if d.is_empty() {
        return None;
    }
    let pos: Vec<usize> = s.match_indices(d).map(|(i, _)| i).collect();
    if from_end { pos.iter().rev().nth(index).copied() } else { pos.get(index).copied() }
}

/// `index` argument of the delimiter functions: a number, or `{n, RelativePosition.FromEnd}`.
fn delim_index(v: &Value) -> (usize, bool) {
    match v {
        Value::Number(n) => (n.max(0.0) as usize, false),
        Value::List(l) => (l.first().and_then(Value::as_number).unwrap_or(0.0).max(0.0) as usize, l.get(1).and_then(Value::as_number) == Some(1.0)),
        _ => (0, false),
    }
}

fn proper(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start = true;
    for c in s.chars() {
        if c.is_alphabetic() {
            if start {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            start = false;
        } else {
            out.push(c);
            start = !c.is_alphanumeric() && c != '\'';
        }
    }
    out
}

fn trim_set(a: &[Value]) -> Option<Vec<char>> {
    match arg(a, 1) {
        Value::Text(t) => Some(t.chars().collect()),
        Value::List(l) => Some(l.iter().filter_map(Value::as_text).flat_map(|s| s.chars().collect::<Vec<_>>()).collect()),
        _ => None,
    }
}

pub fn call(_ev: &Evaluator, name: &str, a: &[Value]) -> Option<Result<Value>> {
    let t = |f: fn(&str) -> String| -> Result<Value> { Ok(text_or_null(a, name)?.map(|s| Value::text(f(&s))).unwrap_or(Value::Null)) };
    let r = match name {
        "Text.From" => Ok(text_from(arg(a, 0))),
        "Text.Upper" => t(|s| s.to_uppercase()),
        "Text.Lower" => t(|s| s.to_lowercase()),
        "Text.Proper" => t(proper),
        "Text.Reverse" => t(|s| s.chars().rev().collect()),
        "Text.Clean" => t(|s| s.chars().filter(|c| !c.is_control()).collect()),
        "Text.Trim" | "Text.TrimStart" | "Text.TrimEnd" => (|| {
            let Some(s) = text_or_null(a, name)? else { return Ok(Value::Null) };
            let set = trim_set(a);
            let pred = |c: char| match &set {
                Some(cs) => cs.contains(&c),
                None => c.is_whitespace(),
            };
            Ok(Value::text(match name {
                "Text.Trim" => s.trim_matches(pred),
                "Text.TrimStart" => s.trim_start_matches(pred),
                _ => s.trim_end_matches(pred),
            }))
        })(),
        "Text.Length" => text_or_null(a, name).map(|s| s.map(|s| Value::Number(s.chars().count() as f64)).unwrap_or(Value::Null)),
        "Text.Start" | "Text.End" => (|| {
            let Some(s) = text_or_null(a, name)? else { return Ok(Value::Null) };
            let n = count_arg(a, 1, name)?;
            let c = chars(&s);
            Ok(Value::text(if name == "Text.Start" { slice(&c, 0, n) } else { slice(&c, c.len().saturating_sub(n), n) }))
        })(),
        "Text.Middle" | "Text.Range" => (|| {
            let Some(s) = text_or_null(a, name)? else { return Ok(Value::Null) };
            let c = chars(&s);
            let from = count_arg(a, 1, name)?;
            if name == "Text.Range" && from > c.len() {
                return err("The offset is beyond the end of the text.");
            }
            let n = if arg(a, 2).is_null() { usize::MAX } else { count_arg(a, 2, name)? };
            Ok(Value::text(slice(&c, from, n)))
        })(),
        "Text.At" => (|| {
            let s = text_arg(a, 0, name)?;
            let i = count_arg(a, 1, name)?;
            s.chars().nth(i).map(|c| Value::text(c.to_string())).ok_or_else(|| crate::Error::Eval("The index is beyond the end of the text.".into()))
        })(),
        "Text.BeforeDelimiter" | "Text.AfterDelimiter" => (|| {
            let Some(s) = text_or_null(a, name)? else { return Ok(Value::Null) };
            let d = text_arg(a, 1, name)?;
            let (i, from_end) = delim_index(arg(a, 2));
            Ok(Value::text(match nth_delim(&s, &d, i, from_end) {
                Some(p) if name == "Text.BeforeDelimiter" => s.get(..p).unwrap_or_default().to_string(),
                Some(p) => s.get(p + d.len()..).unwrap_or_default().to_string(),
                None if name == "Text.BeforeDelimiter" => s.clone(),
                None => String::new(),
            }))
        })(),
        "Text.BetweenDelimiters" => (|| {
            let Some(s) = text_or_null(a, name)? else { return Ok(Value::Null) };
            let d1 = text_arg(a, 1, name)?;
            let d2 = text_arg(a, 2, name)?;
            let (i, from_end) = delim_index(arg(a, 3));
            let (j, _) = delim_index(arg(a, 4));
            let Some(p) = nth_delim(&s, &d1, i, from_end) else { return Ok(Value::text("")) };
            let rest = s.get(p + d1.len()..).unwrap_or_default();
            Ok(Value::text(match nth_delim(rest, &d2, j, false) {
                Some(q) => rest.get(..q).unwrap_or_default(),
                None => rest,
            }))
        })(),
        "Text.Contains" | "Text.StartsWith" | "Text.EndsWith" => (|| {
            let Some(s) = text_or_null(a, name)? else { return Ok(Value::Null) };
            let sub = text_arg(a, 1, name)?;
            let ci =
                matches!(arg(a, 2), Value::Function(f) if matches!(&**f, crate::m::eval::Function::Builtin(n) if n == "Comparer.OrdinalIgnoreCase"));
            let (s, sub) = if ci { (s.to_lowercase(), sub.to_lowercase()) } else { (s, sub) };
            Ok(Value::Logical(match name {
                "Text.Contains" => s.contains(&sub),
                "Text.StartsWith" => s.starts_with(&sub),
                _ => s.ends_with(&sub),
            }))
        })(),
        "Text.Replace" => (|| {
            let Some(s) = text_or_null(a, name)? else { return Ok(Value::Null) };
            let old = text_arg(a, 1, name)?;
            let new = text_arg(a, 2, name)?;
            Ok(Value::text(if old.is_empty() { s } else { s.replace(&old, &new) }))
        })(),
        "Text.ReplaceRange" | "Text.RemoveRange" | "Text.Insert" => (|| {
            let s = text_arg(a, 0, name)?;
            let c = chars(&s);
            let off = count_arg(a, 1, name)?.min(c.len());
            let (n, new) = match name {
                "Text.Insert" => (0, text_arg(a, 2, name)?),
                "Text.RemoveRange" => (if arg(a, 2).is_null() { 1 } else { count_arg(a, 2, name)? }, String::new()),
                _ => (count_arg(a, 2, name)?, text_arg(a, 3, name)?),
            };
            Ok(Value::text(format!("{}{}{}", slice(&c, 0, off), new, slice(&c, off.saturating_add(n), usize::MAX))))
        })(),
        "Text.Split" => (|| {
            let s = text_arg(a, 0, name)?;
            let d = text_arg(a, 1, name)?;
            Ok(Value::list(super::table::split_text(&s, &d, false).into_iter().map(Value::text).collect()))
        })(),
        "Text.SplitAny" => (|| {
            let s = text_arg(a, 0, name)?;
            let d: Vec<char> = text_arg(a, 1, name)?.chars().collect();
            Ok(Value::list(s.split(|c| d.contains(&c)).map(Value::text).collect()))
        })(),
        "Text.Combine" => (|| {
            let l = list_arg(a, 0, name)?;
            let sep = arg(a, 1).as_text().unwrap_or_default().to_string();
            let parts: Vec<String> = l.iter().filter(|v| !v.is_null()).map(|v| as_text(v).unwrap_or_default()).collect();
            Ok(Value::text(parts.join(&sep)))
        })(),
        "Text.PadStart" | "Text.PadEnd" => (|| {
            let Some(s) = text_or_null(a, name)? else { return Ok(Value::Null) };
            let n = count_arg(a, 1, name)?.min(1_000_000);
            let pad = arg(a, 2).as_text().and_then(|p| p.chars().next()).unwrap_or(' ');
            let len = s.chars().count();
            let fill: String = std::iter::repeat_n(pad, n.saturating_sub(len)).collect();
            Ok(Value::text(if name == "Text.PadStart" { format!("{fill}{s}") } else { format!("{s}{fill}") }))
        })(),
        "Text.PositionOf" => (|| {
            let s = text_arg(a, 0, name)?;
            let sub = text_arg(a, 1, name)?;
            let occ = opt_num(a, 2).unwrap_or(0.0);
            let c_idx = |b: usize| s.get(..b).map(|x| x.chars().count() as f64).unwrap_or(-1.0);
            let pos: Vec<usize> = s.match_indices(sub.as_str()).map(|(i, _)| i).collect();
            Ok(match occ as u8 {
                1 => Value::Number(pos.last().map(|p| c_idx(*p)).unwrap_or(-1.0)),
                2 => Value::list(pos.iter().map(|p| Value::Number(c_idx(*p))).collect()),
                _ => Value::Number(pos.first().map(|p| c_idx(*p)).unwrap_or(-1.0)),
            })
        })(),
        "Text.PositionOfAny" => (|| {
            let s = text_arg(a, 0, name)?;
            let set: Vec<char> = match arg(a, 1) {
                Value::List(l) => l.iter().filter_map(Value::as_text).flat_map(|x| x.chars().collect::<Vec<_>>()).collect(),
                _ => vec![],
            };
            Ok(Value::Number(s.chars().position(|c| set.contains(&c)).map(|p| p as f64).unwrap_or(-1.0)))
        })(),
        "Text.Remove" | "Text.Select" => (|| {
            let Some(s) = text_or_null(a, name)? else { return Ok(Value::Null) };
            let set: Vec<char> = match arg(a, 1) {
                Value::Text(t) => t.chars().collect(),
                Value::List(l) => l.iter().filter_map(Value::as_text).flat_map(|x| x.chars().collect::<Vec<_>>()).collect(),
                _ => vec![],
            };
            let keep = name == "Text.Select";
            Ok(Value::text(s.chars().filter(|c| set.contains(c) == keep).collect::<String>()))
        })(),
        "Text.Repeat" => (|| {
            let s = text_arg(a, 0, name)?;
            let n = count_arg(a, 1, name)?;
            if s.len().saturating_mul(n) > 10_000_000 {
                return err("The result is too long.");
            }
            Ok(Value::text(s.repeat(n)))
        })(),
        "Text.Format" => (|| {
            let fmt = text_arg(a, 0, name)?;
            let mut out = fmt.clone();
            match arg(a, 1) {
                Value::List(l) => {
                    for (i, v) in l.iter().enumerate() {
                        out = out.replace(&format!("#{{{i}}}"), &as_text(v).unwrap_or_default());
                    }
                }
                Value::Record(r) => {
                    for (n, v) in r.names.iter().zip(r.values.iter()) {
                        out = out.replace(&format!("#[{n}]"), &as_text(v).unwrap_or_default());
                    }
                }
                _ => {}
            }
            Ok(Value::text(out))
        })(),
        "Text.ToList" => text_arg(a, 0, name).map(|s| Value::list(s.chars().map(|c| Value::text(c.to_string())).collect())),
        "Character.FromNumber" => super::num_arg(a, 0, name).map(|n| Value::text(char::from_u32(n as u32).map(String::from).unwrap_or_default())),
        "Character.ToNumber" => text_arg(a, 0, name).map(|s| Value::Number(s.chars().next().map(|c| c as u32 as f64).unwrap_or(0.0))),
        _ => return None,
    };
    Some(r)
}
