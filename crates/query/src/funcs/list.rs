//! `List.*` functions.

use super::{arg, count_arg, err, func_arg, list_arg, num_arg};
use crate::Result;
use crate::m::eval::{Evaluator, check};
use crate::value::{Value, compare, values_equal};

const MAX_ITEMS: usize = 20_000_000;

fn nums(l: &[Value]) -> Vec<f64> {
    l.iter()
        .filter_map(|v| match v {
            Value::Number(n) => Some(*n),
            _ => None,
        })
        .collect()
}

fn agg(name: &str, l: &[Value]) -> Value {
    let n = nums(l);
    if n.is_empty() {
        return match name {
            "List.Sum" | "List.Product" if l.iter().all(|v| v.is_null()) => Value::Null,
            _ => Value::Null,
        };
    }
    let count = n.len() as f64;
    Value::Number(match name {
        "List.Sum" => n.iter().sum(),
        "List.Product" => n.iter().product(),
        "List.Average" => n.iter().sum::<f64>() / count,
        "List.Median" => {
            let mut s = n.clone();
            s.sort_by(f64::total_cmp);
            let m = s.len() / 2;
            if s.len().is_multiple_of(2) {
                (s.get(m - 1).copied().unwrap_or(0.0) + s.get(m).copied().unwrap_or(0.0)) / 2.0
            } else {
                s.get(m).copied().unwrap_or(0.0)
            }
        }
        _ => {
            if n.len() < 2 {
                return Value::Null;
            }
            let mean = n.iter().sum::<f64>() / count;
            (n.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (count - 1.0)).sqrt()
        }
    })
}

fn extreme(l: &[Value], max: bool) -> Option<Value> {
    let mut best: Option<&Value> = None;
    for v in l.iter().filter(|v| !v.is_null()) {
        best = match best {
            None => Some(v),
            Some(b) => {
                let o = compare(v, b);
                if (max && o.is_gt()) || (!max && o.is_lt()) { Some(v) } else { Some(b) }
            }
        };
    }
    best.cloned()
}

pub fn distinct(l: &[Value]) -> Vec<Value> {
    let mut seen = std::collections::HashSet::new();
    l.iter().filter(|v| seen.insert(super::table::key_of(&[*v]))).cloned().collect()
}

pub fn call(ev: &Evaluator, name: &str, a: &[Value]) -> Option<Result<Value>> {
    let l = || list_arg(a, 0, name);
    let r = match name {
        "List.Sum" | "List.Product" | "List.Average" | "List.Median" | "List.StandardDeviation" => l().map(|l| agg(name, &l)),
        "List.Max" | "List.Min" => l().map(|l| extreme(&l, name == "List.Max").unwrap_or_else(|| arg(a, 1).clone())),
        "List.Count" => l().map(|l| Value::Number(l.len() as f64)),
        "List.NonNullCount" => l().map(|l| Value::Number(l.iter().filter(|v| !v.is_null()).count() as f64)),
        "List.IsEmpty" => l().map(|l| Value::Logical(l.is_empty())),
        "List.Distinct" => l().map(|l| Value::list(distinct(&l))),
        "List.Mode" => l().map(|l| {
            let mut counts: Vec<(Value, usize)> = Vec::new();
            for v in l.iter() {
                match counts.iter_mut().find(|(x, _)| values_equal(x, v)) {
                    Some(e) => e.1 += 1,
                    None => counts.push((v.clone(), 1)),
                }
            }
            counts.into_iter().max_by_key(|(_, c)| *c).map(|(v, _)| v).unwrap_or_default()
        }),
        "List.Sort" => l().and_then(|l| {
            let mut v = l.to_vec();
            match arg(a, 1) {
                f @ Value::Function(_) => {
                    let mut keyed = Vec::new();
                    for x in v {
                        keyed.push((ev.apply1(f, x.clone())?, x));
                    }
                    keyed.sort_by(|x, y| compare(&x.0, &y.0));
                    v = keyed.into_iter().map(|(_, x)| x).collect();
                }
                Value::Number(d) if *d == 1.0 => v.sort_by(|x, y| compare(y, x)),
                _ => v.sort_by(compare),
            }
            Ok(Value::list(v))
        }),
        "List.Reverse" => l().map(|l| Value::list(l.iter().rev().cloned().collect())),
        "List.First" => l().map(|l| l.first().cloned().unwrap_or_else(|| arg(a, 1).clone())),
        "List.Last" => l().map(|l| l.last().cloned().unwrap_or_else(|| arg(a, 1).clone())),
        "List.FirstN" | "List.LastN" | "List.Skip" | "List.RemoveFirstN" | "List.RemoveLastN" => l().and_then(|l| {
            let n = match arg(a, 1) {
                Value::Null => 1,
                Value::Function(_) => {
                    let f = arg(a, 1);
                    let mut k = 0;
                    let it: Box<dyn Iterator<Item = &Value>> =
                        if matches!(name, "List.LastN" | "List.RemoveLastN") { Box::new(l.iter().rev()) } else { Box::new(l.iter()) };
                    for v in it {
                        if matches!(ev.apply1(f, v.clone())?, Value::Logical(true)) {
                            k += 1;
                        } else {
                            break;
                        }
                    }
                    k
                }
                _ => count_arg(a, 1, name)?,
            }
            .min(l.len());
            Ok(Value::list(match name {
                "List.FirstN" => l.get(..n).unwrap_or_default().to_vec(),
                "List.LastN" => l.get(l.len() - n..).unwrap_or_default().to_vec(),
                "List.RemoveLastN" => l.get(..l.len() - n).unwrap_or_default().to_vec(),
                _ => l.get(n..).unwrap_or_default().to_vec(),
            }))
        }),
        "List.Range" => l().and_then(|l| {
            let off = count_arg(a, 1, name)?.min(l.len());
            let n = if arg(a, 2).is_null() { l.len() } else { count_arg(a, 2, name)? };
            Ok(Value::list(l.get(off..off.saturating_add(n).min(l.len())).unwrap_or_default().to_vec()))
        }),
        "List.Contains" => l().map(|l| Value::Logical(l.iter().any(|v| values_equal(v, arg(a, 1))))),
        "List.ContainsAny" | "List.ContainsAll" => l().and_then(|l| {
            let w = list_arg(a, 1, name)?;
            let has = |x: &Value| l.iter().any(|v| values_equal(v, x));
            Ok(Value::Logical(if name == "List.ContainsAny" { w.iter().any(has) } else { w.iter().all(has) }))
        }),
        "List.PositionOf" => l().map(|l| Value::Number(l.iter().position(|v| values_equal(v, arg(a, 1))).map(|p| p as f64).unwrap_or(-1.0))),
        "List.Transform" => l().and_then(|l| {
            let f = func_arg(a, 1, name)?;
            let mut out = Vec::with_capacity(l.len());
            for v in l.iter() {
                ev.tick()?;
                out.push(ev.apply1(&f, v.clone())?);
            }
            Ok(Value::list(out))
        }),
        "List.Select" => l().and_then(|l| {
            let f = func_arg(a, 1, name)?;
            let mut out = Vec::new();
            for v in l.iter() {
                if matches!(check(ev.apply1(&f, v.clone())?)?, Value::Logical(true)) {
                    out.push(v.clone());
                }
            }
            Ok(Value::list(out))
        }),
        "List.MatchesAll" | "List.MatchesAny" => l().and_then(|l| {
            let f = func_arg(a, 1, name)?;
            let mut res = Vec::new();
            for v in l.iter() {
                res.push(matches!(ev.apply1(&f, v.clone())?, Value::Logical(true)));
            }
            Ok(Value::Logical(if name == "List.MatchesAll" { res.iter().all(|x| *x) } else { res.iter().any(|x| *x) }))
        }),
        "List.AllTrue" => l().map(|l| Value::Logical(l.iter().all(|v| matches!(v, Value::Logical(true))))),
        "List.AnyTrue" => l().map(|l| Value::Logical(l.iter().any(|v| matches!(v, Value::Logical(true))))),
        "List.Combine" => {
            l().map(|l| Value::list(l.iter().flat_map(|x| if let Value::List(i) = x { i.to_vec() } else { vec![x.clone()] }).collect()))
        }
        "List.Union" => {
            l().map(|l| Value::list(distinct(&l.iter().flat_map(|x| if let Value::List(i) = x { i.to_vec() } else { vec![] }).collect::<Vec<_>>())))
        }
        "List.Intersect" => l().map(|l| {
            let lists: Vec<Vec<Value>> = l.iter().map(|x| if let Value::List(i) = x { i.to_vec() } else { vec![] }).collect();
            let first = lists.first().cloned().unwrap_or_default();
            Value::list(distinct(&first).into_iter().filter(|v| lists.iter().all(|o| o.iter().any(|x| values_equal(x, v)))).collect())
        }),
        "List.Difference" | "List.RemoveItems" => l().and_then(|l| {
            let w = list_arg(a, 1, name)?;
            Ok(Value::list(l.iter().filter(|v| !w.iter().any(|x| values_equal(x, v))).cloned().collect()))
        }),
        "List.RemoveNulls" => l().map(|l| Value::list(l.iter().filter(|v| !v.is_null()).cloned().collect())),
        "List.Zip" => l().map(|l| {
            let lists: Vec<Vec<Value>> = l.iter().map(|x| if let Value::List(i) = x { i.to_vec() } else { vec![] }).collect();
            let n = lists.iter().map(Vec::len).max().unwrap_or(0);
            Value::list((0..n).map(|i| Value::list(lists.iter().map(|x| x.get(i).cloned().unwrap_or_default()).collect())).collect())
        }),
        "List.Split" => l().and_then(|l| {
            let n = count_arg(a, 1, name)?.max(1);
            Ok(Value::list(l.chunks(n).map(|c| Value::list(c.to_vec())).collect()))
        }),
        "List.Repeat" => l().and_then(|l| {
            let n = count_arg(a, 1, name)?;
            if l.len().saturating_mul(n) > MAX_ITEMS {
                return err("The list is too long.");
            }
            Ok(Value::list((0..n).flat_map(|_| l.iter().cloned()).collect()))
        }),
        "List.Numbers" => (|| {
            let start = num_arg(a, 0, name)?;
            let n = count_arg(a, 1, name)?.min(MAX_ITEMS);
            let step = super::opt_num(a, 2).unwrap_or(1.0);
            Ok(Value::list((0..n).map(|i| Value::Number(start + step * i as f64)).collect()))
        })(),
        "List.Dates" => (|| {
            let n = count_arg(a, 1, name)?.min(MAX_ITEMS);
            let step = match arg(a, 2) {
                Value::Duration(d) => *d,
                _ => 1.0,
            };
            Ok(Value::list(
                (0..n)
                    .map(|i| match arg(a, 0) {
                        Value::Date(d) => Value::Date((*d as f64 + step * i as f64).floor() as i64),
                        Value::DateTime(d) => Value::DateTime(d + step * i as f64),
                        _ => Value::Null,
                    })
                    .collect(),
            ))
        })(),
        "List.Accumulate" => l().and_then(|l| {
            let mut acc = arg(a, 1).clone();
            let f = func_arg(a, 2, name)?;
            for v in l.iter() {
                ev.tick()?;
                acc = ev.apply(&f, vec![acc, v.clone()])?;
            }
            Ok(acc)
        }),
        "List.Generate" => (|| {
            let (init, cond, next) = (func_arg(a, 0, name)?, func_arg(a, 1, name)?, func_arg(a, 2, name)?);
            let sel = arg(a, 3).clone();
            let mut v = ev.apply(&init, vec![])?;
            let mut out = Vec::new();
            while matches!(ev.apply1(&cond, v.clone())?, Value::Logical(true)) {
                ev.tick()?;
                out.push(if sel.is_null() { v.clone() } else { ev.apply1(&sel, v.clone())? });
                if out.len() > MAX_ITEMS {
                    return err("The list is too long.");
                }
                v = ev.apply1(&next, v)?;
            }
            Ok(Value::list(out))
        })(),
        "List.Buffer" => l().map(Value::List),
        _ => return None,
    };
    Some(r)
}
