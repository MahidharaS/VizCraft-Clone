//! `Table.*` functions.

use std::collections::HashMap;
use std::rc::Rc;

use super::convert::{as_text, to_type};
use super::{arg, count_arg, err, func_arg, list_arg, names_arg, native, opt_num, table_arg, text_arg};
use crate::m::eval::{Evaluator, check};
use crate::value::{MType, Table, Value, compare, values_equal};
use crate::{Error, Result};

/// The most rows a table operation may produce.
pub const MAX_ROWS: usize = 20_000_000;

fn col_index(t: &Table, name: &str) -> Result<usize> {
    t.column_index(name).ok_or_else(|| Error::Eval(format!("The column '{name}' of the table wasn't found.")))
}

/// Missing-field handling: 0 error, 1 ignore, 2 use null.
fn missing_mode(v: &Value) -> u8 {
    v.as_number().map(|n| n as u8).unwrap_or(0)
}

/// A hashable key for joins and grouping.
pub fn key_of(vals: &[&Value]) -> String {
    let mut s = String::new();
    for v in vals {
        s.push_str(v.kind());
        s.push('\u{1}');
        s.push_str(&match v {
            Value::Text(t) => t.to_string(),
            other => other.m_literal(),
        });
        s.push('\u{2}');
    }
    s
}

fn unique_name(cols: &[String], name: &str) -> String {
    if !cols.iter().any(|c| c == name) {
        return name.to_string();
    }
    (1..).map(|i| format!("{name}.{i}")).find(|n| !cols.iter().any(|c| c == n)).unwrap_or_default()
}

/// Applies a function to each row (as a record); an error becomes an error value in the cell.
fn per_row(ev: &Evaluator, t: &Table, f: &Value) -> Result<Vec<Value>> {
    let mut out = Vec::with_capacity(t.rows.len());
    for r in 0..t.rows.len() {
        ev.tick()?;
        out.push(match ev.apply1(f, Value::record(t.row_record(r))) {
            Ok(v) => v,
            Err(e) => Value::error(e.message()),
        });
    }
    Ok(out)
}

fn rows_where(ev: &Evaluator, t: &Table, f: &Value) -> Result<Vec<usize>> {
    let mut keep = Vec::new();
    for r in 0..t.rows.len() {
        ev.tick()?;
        match check(ev.apply1(f, Value::record(t.row_record(r)))?)? {
            Value::Logical(true) => keep.push(r),
            Value::Logical(false) | Value::Null => {}
            other => return err(format!("We cannot convert the value {} to type Logical.", other.display())),
        }
    }
    Ok(keep)
}

fn select_rows(t: &Table, rows: &[usize]) -> Table {
    Table { columns: t.columns.clone(), types: t.types.clone(), rows: rows.iter().filter_map(|r| t.rows.get(*r).cloned()).collect() }
}

/// A count argument or a row condition (`Table.FirstN(t, 3)`, `Table.FirstN(t, each [A] > 1)`):
/// the number of leading rows it covers.
fn leading(ev: &Evaluator, t: &Table, v: &Value, from_end: bool) -> Result<usize> {
    match v {
        Value::Number(n) => Ok((n.max(0.0) as usize).min(t.rows.len())),
        Value::Null => Ok(1.min(t.rows.len())),
        f @ Value::Function(_) => {
            let order: Box<dyn Iterator<Item = usize>> = if from_end { Box::new((0..t.rows.len()).rev()) } else { Box::new(0..t.rows.len()) };
            let mut n = 0;
            for r in order {
                match check(ev.apply1(f, Value::record(t.row_record(r)))?)? {
                    Value::Logical(true) => n += 1,
                    _ => break,
                }
            }
            Ok(n)
        }
        other => err(format!("expected a count or a condition, got {}", other.kind())),
    }
}

/// Sort criteria: `"A"`, `{"A", Order.Descending}`, `{{"A", 0}, {"B", 1}}`, or a key function.
enum SortKey {
    Col(usize, bool),
    Func(Value, bool),
}

fn sort_keys(t: &Table, v: &Value) -> Result<Vec<SortKey>> {
    let one = |v: &Value| -> Result<SortKey> {
        match v {
            Value::Text(c) => Ok(SortKey::Col(col_index(t, c)?, false)),
            f @ Value::Function(_) => Ok(SortKey::Func(f.clone(), false)),
            Value::List(l) => {
                let desc = l.get(1).and_then(Value::as_number).is_some_and(|n| n == 1.0);
                match l.first() {
                    Some(Value::Text(c)) => Ok(SortKey::Col(col_index(t, c)?, desc)),
                    Some(f @ Value::Function(_)) => Ok(SortKey::Func(f.clone(), desc)),
                    _ => err("bad sort criteria"),
                }
            }
            _ => err("bad sort criteria"),
        }
    };
    match v {
        Value::List(l) if l.first().is_some_and(|x| matches!(x, Value::List(_))) => l.iter().map(one).collect(),
        Value::List(l) if l.len() == 2 && l.get(1).is_some_and(|x| matches!(x, Value::Number(_))) => Ok(vec![one(v)?]),
        Value::List(l) => l.iter().map(one).collect(),
        other => Ok(vec![one(other)?]),
    }
}

fn sorted_rows(ev: &Evaluator, t: &Table, keys: &[SortKey]) -> Result<Vec<usize>> {
    let mut keyed: Vec<(Vec<Value>, usize)> = Vec::with_capacity(t.rows.len());
    for r in 0..t.rows.len() {
        let mut k = Vec::with_capacity(keys.len());
        for sk in keys {
            k.push(match sk {
                SortKey::Col(c, _) => t.cell(r, *c).clone(),
                SortKey::Func(f, _) => ev.apply1(f, Value::record(t.row_record(r)))?,
            });
        }
        keyed.push((k, r));
    }
    keyed.sort_by(|a, b| {
        for (i, sk) in keys.iter().enumerate() {
            let desc = matches!(sk, SortKey::Col(_, true) | SortKey::Func(_, true));
            let o = compare(&a.0[i], &b.0[i]);
            let o = if desc { o.reverse() } else { o };
            if o.is_ne() {
                return o;
            }
        }
        std::cmp::Ordering::Equal
    });
    Ok(keyed.into_iter().map(|(_, r)| r).collect())
}

/// Pairs given as `{a, b}` or `{{a, b}, {c, d}}`.
fn pairs(v: &Value) -> Vec<Vec<Value>> {
    match v {
        Value::List(l) if l.first().is_some_and(|x| matches!(x, Value::List(_))) => {
            l.iter().filter_map(|x| if let Value::List(p) = x { Some(p.to_vec()) } else { None }).collect()
        }
        Value::List(l) => vec![l.to_vec()],
        _ => vec![],
    }
}

pub fn select_columns(v: &Value, names: &[String], opt: bool) -> Result<Value> {
    let Value::Table(t) = v else { return err("expected a table") };
    let mut cols = Vec::new();
    let mut idx = Vec::new();
    for n in names {
        match t.column_index(n) {
            Some(i) => {
                cols.push(n.clone());
                idx.push(Some(i));
            }
            None if opt => {
                cols.push(n.clone());
                idx.push(None);
            }
            None => return err(format!("The column '{n}' of the table wasn't found.")),
        }
    }
    let types = idx.iter().map(|i| i.and_then(|i| t.types.get(i).copied()).unwrap_or_default()).collect();
    let rows = t.rows.iter().map(|r| idx.iter().map(|i| i.and_then(|i| r.get(i).cloned()).unwrap_or_default()).collect()).collect();
    Ok(Value::table(Table { columns: cols, types, rows }))
}

pub fn combine(tables: &[Value]) -> Result<Value> {
    let mut columns: Vec<String> = Vec::new();
    let mut types: Vec<MType> = Vec::new();
    for t in tables {
        let Value::Table(t) = t else { return err("Table.Combine: expected a list of tables") };
        for (i, c) in t.columns.iter().enumerate() {
            if !columns.contains(c) {
                columns.push(c.clone());
                types.push(t.types.get(i).copied().unwrap_or_default());
            }
        }
    }
    let mut rows = Vec::new();
    for t in tables {
        let Value::Table(t) = t else { continue };
        let map: Vec<Option<usize>> = columns.iter().map(|c| t.column_index(c)).collect();
        for r in &t.rows {
            rows.push(map.iter().map(|i| i.and_then(|i| r.get(i).cloned()).unwrap_or_default()).collect());
            if rows.len() > MAX_ROWS {
                return err("The result has too many rows.");
            }
        }
    }
    Ok(Value::table(Table { columns, types, rows }))
}

/// Header text for promoted values: empty becomes `Column{n}`, repeats get `_1`, `_2`.
fn header_names(vals: &[Value]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (i, v) in vals.iter().enumerate() {
        let base = match as_text(v) {
            Some(s) if !s.is_empty() => s,
            _ => format!("Column{}", i + 1),
        };
        let mut name = base.clone();
        let mut k = 1;
        while out.contains(&name) {
            name = format!("{base}_{k}");
            k += 1;
        }
        out.push(name);
    }
    out
}

pub fn call(ev: &Evaluator, name: &str, a: &[Value]) -> Option<Result<Value>> {
    let r = match name {
        "Table.PromoteHeaders" => promote(a),
        "Table.DemoteHeaders" => demote(a),
        "Table.TransformColumnTypes" => transform_types(a),
        "Table.RenameColumns" => rename(a),
        "Table.RemoveColumns" => remove_columns(a),
        "Table.SelectColumns" => {
            table_arg(a, 0, name).and_then(|t| select_columns(&Value::Table(t), &names_arg(arg(a, 1))?, missing_mode(arg(a, 2)) != 0))
        }
        "Table.ReorderColumns" => reorder(a),
        "Table.DuplicateColumn" => duplicate(a),
        "Table.AddColumn" => add_column(ev, a),
        "Table.AddIndexColumn" => add_index(a),
        "Table.AddRankColumn" => add_rank(ev, a),
        "Table.SelectRows" => table_arg(a, 0, name).and_then(|t| Ok(Value::table(select_rows(&t, &rows_where(ev, &t, &func_arg(a, 1, name)?)?)))),
        "Table.FirstN" => {
            table_arg(a, 0, name).and_then(|t| Ok(Value::table(select_rows(&t, &(0..leading(ev, &t, arg(a, 1), false)?).collect::<Vec<_>>()))))
        }
        "Table.LastN" => table_arg(a, 0, name).and_then(|t| {
            let n = leading(ev, &t, arg(a, 1), true)?;
            Ok(Value::table(select_rows(&t, &(t.rows.len() - n..t.rows.len()).collect::<Vec<_>>())))
        }),
        "Table.Skip" | "Table.RemoveFirstN" => table_arg(a, 0, name).and_then(|t| {
            let n = leading(ev, &t, arg(a, 1), false)?;
            Ok(Value::table(select_rows(&t, &(n..t.rows.len()).collect::<Vec<_>>())))
        }),
        "Table.RemoveLastN" => table_arg(a, 0, name).and_then(|t| {
            let n = leading(ev, &t, arg(a, 1), true)?;
            Ok(Value::table(select_rows(&t, &(0..t.rows.len() - n).collect::<Vec<_>>())))
        }),
        "Table.Range" => table_arg(a, 0, name).and_then(|t| {
            let off = count_arg(a, 1, name)?.min(t.rows.len());
            let n = if arg(a, 2).is_null() { t.rows.len() } else { count_arg(a, 2, name)? };
            Ok(Value::table(select_rows(&t, &(off..(off.saturating_add(n)).min(t.rows.len())).collect::<Vec<_>>())))
        }),
        "Table.RemoveRows" => table_arg(a, 0, name).and_then(|t| {
            let off = count_arg(a, 1, name)?;
            let n = if arg(a, 2).is_null() { 1 } else { count_arg(a, 2, name)? };
            Ok(Value::table(select_rows(&t, &(0..t.rows.len()).filter(|r| *r < off || *r >= off.saturating_add(n)).collect::<Vec<_>>())))
        }),
        "Table.AlternateRows" => table_arg(a, 0, name).and_then(|t| {
            let off = count_arg(a, 1, name)?;
            let skip = count_arg(a, 2, name)?;
            let take = count_arg(a, 3, name)?;
            let period = (skip + take).max(1);
            Ok(Value::table(select_rows(&t, &(0..t.rows.len()).filter(|r| *r < off || (r - off) % period >= skip).collect::<Vec<_>>())))
        }),
        "Table.ReverseRows" => table_arg(a, 0, name).map(|t| Value::table(select_rows(&t, &(0..t.rows.len()).rev().collect::<Vec<_>>()))),
        "Table.Repeat" => table_arg(a, 0, name).and_then(|t| {
            let n = count_arg(a, 1, name)?;
            if t.rows.len().saturating_mul(n) > MAX_ROWS {
                return err("The result has too many rows.");
            }
            let rows: Vec<usize> = (0..n).flat_map(|_| 0..t.rows.len()).collect();
            Ok(Value::table(select_rows(&t, &rows)))
        }),
        "Table.Distinct" => distinct(a),
        "Table.RemoveRowsWithErrors" | "Table.SelectRowsWithErrors" => table_arg(a, 0, name).and_then(|t| {
            let cols: Vec<usize> = match arg(a, 1) {
                Value::Null => (0..t.columns.len()).collect(),
                v => names_arg(v)?.iter().map(|c| col_index(&t, c)).collect::<Result<_>>()?,
            };
            let want_errors = name == "Table.SelectRowsWithErrors";
            let rows: Vec<usize> = (0..t.rows.len()).filter(|r| cols.iter().any(|c| t.cell(*r, *c).is_error()) == want_errors).collect();
            Ok(Value::table(select_rows(&t, &rows)))
        }),
        "Table.RemoveMatchingRows" => table_arg(a, 0, name).and_then(|t| {
            let pats = list_arg(a, 1, name)?;
            let rows: Vec<usize> = (0..t.rows.len())
                .filter(|r| {
                    !pats.iter().any(|p| match p {
                        Value::Record(rec) => {
                            rec.names.iter().zip(rec.values.iter()).all(|(n, v)| t.column_index(n).is_some_and(|c| values_equal(t.cell(*r, c), v)))
                        }
                        _ => false,
                    })
                })
                .collect();
            Ok(Value::table(select_rows(&t, &rows)))
        }),
        "Table.Sort" => table_arg(a, 0, name).and_then(|t| Ok(Value::table(select_rows(&t, &sorted_rows(ev, &t, &sort_keys(&t, arg(a, 1))?)?)))),
        "Table.ReplaceValue" => replace_value(ev, a),
        "Table.ReplaceErrorValues" => table_arg(a, 0, name).and_then(|t| {
            let mut t = (*t).clone();
            for p in pairs(arg(a, 1)) {
                let Some(Value::Text(c)) = p.first() else { continue };
                let ci = col_index(&t, c)?;
                let v = p.get(1).cloned().unwrap_or_default();
                for row in &mut t.rows {
                    if let Some(cell) = row.get_mut(ci).filter(|x| x.is_error()) {
                        *cell = v.clone();
                    }
                }
            }
            Ok(Value::table(t))
        }),
        "Table.TransformColumns" => transform_columns(ev, a),
        "Table.FillDown" | "Table.FillUp" => fill(a, name == "Table.FillUp"),
        "Table.Group" => group(ev, a),
        "Table.Pivot" => pivot(ev, a),
        "Table.Unpivot" => unpivot(a, false),
        "Table.UnpivotOtherColumns" => unpivot(a, true),
        "Table.Transpose" => transpose(a),
        "Table.NestedJoin" => nested_join(a),
        "Table.Join" => flat_join(a),
        "Table.ExpandTableColumn" => expand_table(a),
        "Table.ExpandRecordColumn" => expand_record(a),
        "Table.ExpandListColumn" => expand_list(a),
        "Table.AggregateTableColumn" => aggregate_table_column(ev, a),
        "Table.Combine" => list_arg(a, 0, name).and_then(|l| combine(&l)),
        "Table.SplitColumn" => split_column(ev, a),
        "Table.CombineColumns" => combine_columns(ev, a),
        "Table.PrefixColumns" => table_arg(a, 0, name).and_then(|t| {
            let p = text_arg(a, 1, name)?;
            let mut t = (*t).clone();
            t.columns = t.columns.iter().map(|c| format!("{p}.{c}")).collect();
            Ok(Value::table(t))
        }),
        "Table.TransformColumnNames" => table_arg(a, 0, name).and_then(|t| {
            let f = func_arg(a, 1, name)?;
            let mut t = (*t).clone();
            let mut names = Vec::new();
            for c in &t.columns {
                names.push(check(ev.apply1(&f, Value::text(c))?)?.as_text().map(str::to_string).unwrap_or_else(|| c.clone()));
            }
            t.columns = names;
            Ok(Value::table(t))
        }),
        "Table.HasColumns" => {
            table_arg(a, 0, name).and_then(|t| Ok(Value::Logical(names_arg(arg(a, 1))?.iter().all(|c| t.column_index(c).is_some()))))
        }
        "Table.Column" => table_arg(a, 0, name).and_then(|t| crate::m::eval::field(&Value::Table(t), &text_arg(a, 1, name)?, false)),
        "Table.ColumnNames" => table_arg(a, 0, name).map(|t| Value::list(t.columns.iter().map(Value::text).collect())),
        "Table.ColumnCount" => table_arg(a, 0, name).map(|t| Value::Number(t.columns.len() as f64)),
        "Table.RowCount" => table_arg(a, 0, name).map(|t| Value::Number(t.rows.len() as f64)),
        "Table.IsEmpty" => table_arg(a, 0, name).map(|t| Value::Logical(t.rows.is_empty())),
        "Table.ToRows" => table_arg(a, 0, name).map(|t| Value::list(t.rows.iter().map(|r| Value::list(r.clone())).collect())),
        "Table.ToRecords" => table_arg(a, 0, name).map(|t| Value::list((0..t.rows.len()).map(|r| Value::record(t.row_record(r))).collect())),
        "Table.ToColumns" => table_arg(a, 0, name).map(|t| {
            Value::list((0..t.columns.len()).map(|c| Value::list(t.rows.iter().map(|r| r.get(c).cloned().unwrap_or_default()).collect())).collect())
        }),
        "Table.ToList" => table_arg(a, 0, name).and_then(|t| {
            let f = arg(a, 1).clone();
            let mut out = Vec::new();
            for r in &t.rows {
                out.push(if f.is_null() {
                    Value::text(r.iter().map(|v| as_text(v).unwrap_or_default()).collect::<Vec<_>>().join(","))
                } else {
                    ev.apply1(&f, Value::list(r.clone()))?
                });
            }
            Ok(Value::list(out))
        }),
        "Table.TransformRows" => table_arg(a, 0, name).and_then(|t| {
            let f = func_arg(a, 1, name)?;
            let mut out = Vec::new();
            for r in 0..t.rows.len() {
                out.push(ev.apply1(&f, Value::record(t.row_record(r)))?);
            }
            Ok(Value::list(out))
        }),
        "Table.First" | "Table.Last" => table_arg(a, 0, name).map(|t| {
            let r = if name == "Table.First" { 0 } else { t.rows.len().saturating_sub(1) };
            if t.rows.is_empty() { arg(a, 1).clone() } else { Value::record(t.row_record(r)) }
        }),
        "Table.Max" | "Table.Min" => table_arg(a, 0, name).and_then(|t| {
            let keys = sort_keys(&t, arg(a, 1))?;
            let order = sorted_rows(ev, &t, &keys)?;
            let pick = if name == "Table.Max" { order.last() } else { order.first() };
            Ok(pick.map(|r| Value::record(t.row_record(*r))).unwrap_or_else(|| arg(a, 2).clone()))
        }),
        "Table.FromRows" => from_rows(a),
        "Table.FromRecords" => from_records(a),
        "Table.FromColumns" => from_columns(a),
        "Table.FromList" => from_list(ev, a),
        "Table.FromValue" => Ok(match arg(a, 0) {
            Value::List(l) => Value::table(Table::new(vec!["Value".into()], l.iter().map(|v| vec![v.clone()]).collect())),
            Value::Record(r) => Value::table(Table::new(
                vec!["Name".into(), "Value".into()],
                r.names.iter().zip(r.values.iter()).map(|(n, v)| vec![Value::text(n), v.clone()]).collect(),
            )),
            v => Value::table(Table::new(vec!["Value".into()], vec![vec![v.clone()]])),
        }),
        "#table" => hash_table(a),
        "Table.Buffer" => table_arg(a, 0, name).map(Value::Table),
        "Table.Profile" => table_arg(a, 0, name).map(|t| crate::profile::profile_table(&t)),
        "Table.Schema" => table_arg(a, 0, name).map(|t| {
            let rows = t
                .columns
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    vec![
                        Value::text(c),
                        Value::Number(i as f64),
                        Value::text(t.types.get(i).copied().unwrap_or_default().m().trim_start_matches("type ")),
                        Value::Logical(true),
                    ]
                })
                .collect();
            Value::table(Table::new(vec!["Name".into(), "Position".into(), "TypeName".into(), "IsNullable".into()], rows))
        }),
        _ => return None,
    };
    Some(r)
}

fn promote(a: &[Value]) -> Result<Value> {
    let t = table_arg(a, 0, "Table.PromoteHeaders")?;
    let Some(first) = t.rows.first() else { return Ok(Value::Table(t)) };
    let columns = header_names(first);
    let n = columns.len();
    Ok(Value::table(Table { columns, types: vec![MType::Any; n], rows: t.rows.iter().skip(1).cloned().collect() }))
}

fn demote(a: &[Value]) -> Result<Value> {
    let t = table_arg(a, 0, "Table.DemoteHeaders")?;
    let n = t.columns.len();
    let mut rows = vec![t.columns.iter().map(Value::text).collect::<Vec<_>>()];
    rows.extend(t.rows.iter().cloned());
    Ok(Value::table(Table { columns: (1..=n).map(|i| format!("Column{i}")).collect(), types: vec![MType::Any; n], rows }))
}

fn transform_types(a: &[Value]) -> Result<Value> {
    let mut t = (*table_arg(a, 0, "Table.TransformColumnTypes")?).clone();
    for p in pairs(arg(a, 1)) {
        let (Some(Value::Text(c)), Some(Value::Type(ty))) = (p.first(), p.get(1)) else { return err("expected {{\"Column\", type}} pairs") };
        let ci = col_index(&t, c)?;
        for row in &mut t.rows {
            if let Some(cell) = row.get_mut(ci) {
                *cell = to_type(cell, *ty);
            }
        }
        if let Some(slot) = t.types.get_mut(ci) {
            *slot = *ty;
        }
    }
    Ok(Value::table(t))
}

fn rename(a: &[Value]) -> Result<Value> {
    let mut t = (*table_arg(a, 0, "Table.RenameColumns")?).clone();
    let mode = missing_mode(arg(a, 2));
    for p in pairs(arg(a, 1)) {
        let (Some(Value::Text(old)), Some(Value::Text(new))) = (p.first(), p.get(1)) else { return err("expected {{\"Old\", \"New\"}} pairs") };
        match t.column_index(old) {
            Some(i) => {
                if t.columns.iter().enumerate().any(|(j, c)| j != i && **c == **new) {
                    return err(format!("The column '{new}' already exists."));
                }
                if let Some(c) = t.columns.get_mut(i) {
                    *c = new.to_string();
                }
            }
            None if mode != 0 => {}
            None => return err(format!("The column '{old}' of the table wasn't found.")),
        }
    }
    Ok(Value::table(t))
}

fn remove_columns(a: &[Value]) -> Result<Value> {
    let t = table_arg(a, 0, "Table.RemoveColumns")?;
    let drop = names_arg(arg(a, 1))?;
    if missing_mode(arg(a, 2)) == 0
        && let Some(m) = drop.iter().find(|c| t.column_index(c).is_none())
    {
        return err(format!("The column '{m}' of the table wasn't found."));
    }
    let keep: Vec<String> = t.columns.iter().filter(|c| !drop.contains(c)).cloned().collect();
    select_columns(&Value::Table(t), &keep, false)
}

fn reorder(a: &[Value]) -> Result<Value> {
    let t = table_arg(a, 0, "Table.ReorderColumns")?;
    let order = names_arg(arg(a, 1))?;
    let listed: Vec<String> = order.iter().filter(|c| t.column_index(c).is_some()).cloned().collect();
    // Listed columns take the slots the listed columns had, in the new order.
    let mut it = listed.iter();
    let cols: Vec<String> =
        t.columns.iter().map(|c| if listed.contains(c) { it.next().cloned().unwrap_or_else(|| c.clone()) } else { c.clone() }).collect();
    select_columns(&Value::Table(t), &cols, false)
}

fn duplicate(a: &[Value]) -> Result<Value> {
    let mut t = (*table_arg(a, 0, "Table.DuplicateColumn")?).clone();
    let src = text_arg(a, 1, "Table.DuplicateColumn")?;
    let new = text_arg(a, 2, "Table.DuplicateColumn")?;
    let ci = col_index(&t, &src)?;
    if t.column_index(&new).is_some() {
        return err(format!("The column '{new}' already exists."));
    }
    t.columns.push(new);
    t.types.push(match arg(a, 3) {
        Value::Type(ty) => *ty,
        _ => t.types.get(ci).copied().unwrap_or_default(),
    });
    for r in &mut t.rows {
        let v = r.get(ci).cloned().unwrap_or_default();
        r.push(v);
    }
    Ok(Value::table(t))
}

fn add_column(ev: &Evaluator, a: &[Value]) -> Result<Value> {
    let mut t = (*table_arg(a, 0, "Table.AddColumn")?).clone();
    let name = text_arg(a, 1, "Table.AddColumn")?;
    if t.column_index(&name).is_some() {
        return err(format!("The field '{name}' already exists in the record."));
    }
    let f = func_arg(a, 2, "Table.AddColumn")?;
    let ty = match arg(a, 3) {
        Value::Type(ty) => *ty,
        _ => MType::Any,
    };
    let vals = per_row(ev, &t, &f)?;
    for (r, v) in t.rows.iter_mut().zip(vals) {
        r.push(if ty == MType::Any { v } else { to_type(&v, ty) });
    }
    t.columns.push(name);
    t.types.push(ty);
    Ok(Value::table(t))
}

fn add_index(a: &[Value]) -> Result<Value> {
    let mut t = (*table_arg(a, 0, "Table.AddIndexColumn")?).clone();
    let name = text_arg(a, 1, "Table.AddIndexColumn")?;
    let start = opt_num(a, 2).unwrap_or(0.0);
    let step = opt_num(a, 3).unwrap_or(1.0);
    for (i, r) in t.rows.iter_mut().enumerate() {
        r.push(Value::Number(start + step * i as f64));
    }
    t.columns.push(name);
    t.types.push(match arg(a, 4) {
        Value::Type(ty) => *ty,
        _ => MType::Int64,
    });
    Ok(Value::table(t))
}

fn add_rank(ev: &Evaluator, a: &[Value]) -> Result<Value> {
    let mut t = (*table_arg(a, 0, "Table.AddRankColumn")?).clone();
    let name = text_arg(a, 1, "Table.AddRankColumn")?;
    let keys = sort_keys(&t, arg(a, 2))?;
    let order = sorted_rows(ev, &t, &keys)?;
    let mut ranks = vec![0usize; t.rows.len()];
    let key_vals = |r: usize| -> Vec<Value> {
        keys.iter()
            .map(|k| match k {
                SortKey::Col(c, _) => t.cell(r, *c).clone(),
                SortKey::Func(..) => Value::Null,
            })
            .collect()
    };
    let mut prev: Option<Vec<Value>> = None;
    let mut rank = 0;
    for (i, r) in order.iter().enumerate() {
        let k = key_vals(*r);
        if prev.as_ref() != Some(&k) {
            rank = i + 1;
        }
        if let Some(slot) = ranks.get_mut(*r) {
            *slot = rank;
        }
        prev = Some(k);
    }
    for (r, row) in t.rows.iter_mut().enumerate() {
        row.push(Value::Number(ranks.get(r).copied().unwrap_or(0) as f64));
    }
    t.columns.push(name);
    t.types.push(MType::Int64);
    Ok(Value::table(t))
}

fn distinct(a: &[Value]) -> Result<Value> {
    let t = table_arg(a, 0, "Table.Distinct")?;
    let cols: Vec<usize> = match arg(a, 1) {
        Value::Null => (0..t.columns.len()).collect(),
        Value::List(l) if l.first().is_some_and(|x| matches!(x, Value::List(_))) => {
            let mut out = Vec::new();
            for p in l.iter() {
                if let Value::List(p) = p
                    && let Some(Value::Text(c)) = p.first()
                {
                    out.push(col_index(&t, c)?);
                }
            }
            out
        }
        v => names_arg(v)?.iter().map(|c| col_index(&t, c)).collect::<Result<_>>()?,
    };
    let mut seen = std::collections::HashSet::new();
    let rows: Vec<usize> = (0..t.rows.len()).filter(|r| seen.insert(key_of(&cols.iter().map(|c| t.cell(*r, *c)).collect::<Vec<_>>()))).collect();
    Ok(Value::table(select_rows(&t, &rows)))
}

fn replace_value(ev: &Evaluator, a: &[Value]) -> Result<Value> {
    let mut t = (*table_arg(a, 0, "Table.ReplaceValue")?).clone();
    let old = arg(a, 1).clone();
    let new = arg(a, 2).clone();
    let replacer = func_arg(a, 3, "Table.ReplaceValue")?;
    let cols: Vec<usize> = names_arg(arg(a, 4))?.iter().map(|c| col_index(&t, c)).collect::<Result<_>>()?;
    for r in &mut t.rows {
        for c in &cols {
            if let Some(cell) = r.get_mut(*c) {
                if cell.is_error() {
                    continue;
                }
                *cell = match ev.apply(&replacer, vec![cell.clone(), old.clone(), new.clone()]) {
                    Ok(v) => v,
                    Err(e) => Value::error(e.message()),
                };
            }
        }
    }
    Ok(Value::table(t))
}

fn transform_columns(ev: &Evaluator, a: &[Value]) -> Result<Value> {
    let mut t = (*table_arg(a, 0, "Table.TransformColumns")?).clone();
    let specs = pairs(arg(a, 1));
    let mode = missing_mode(arg(a, 3));
    let mut done = vec![false; t.columns.len()];
    for p in specs {
        let (Some(Value::Text(c)), Some(f)) = (p.first(), p.get(1)) else { return err("expected {{\"Column\", function}} pairs") };
        let ci = match t.column_index(c) {
            Some(i) => i,
            None if mode != 0 => continue,
            None => return err(format!("The column '{c}' of the table wasn't found.")),
        };
        let ty = match p.get(2) {
            Some(Value::Type(ty)) => Some(*ty),
            _ => None,
        };
        for r in &mut t.rows {
            if let Some(cell) = r.get_mut(ci) {
                if cell.is_error() {
                    continue;
                }
                ev.tick()?;
                let v = match ev.apply1(f, cell.clone()) {
                    Ok(v) => v,
                    Err(e) => Value::error(e.message()),
                };
                *cell = match ty {
                    Some(ty) if ty != MType::Any => to_type(&v, ty),
                    _ => v,
                };
            }
        }
        if let Some(slot) = t.types.get_mut(ci) {
            *slot = ty.unwrap_or(MType::Any);
        }
        if let Some(d) = done.get_mut(ci) {
            *d = true;
        }
    }
    // The default transformation applies to the columns not listed.
    if let f @ Value::Function(_) = arg(a, 2) {
        for (ci, d) in done.iter().enumerate() {
            if *d {
                continue;
            }
            for r in &mut t.rows {
                if let Some(cell) = r.get_mut(ci) {
                    *cell = ev.apply1(f, cell.clone()).unwrap_or_else(|e| Value::error(e.message()));
                }
            }
        }
    }
    Ok(Value::table(t))
}

fn fill(a: &[Value], up: bool) -> Result<Value> {
    let mut t = (*table_arg(a, 0, "Table.FillDown")?).clone();
    let cols: Vec<usize> = names_arg(arg(a, 1))?.iter().map(|c| col_index(&t, c)).collect::<Result<_>>()?;
    let n = t.rows.len();
    for c in cols {
        let mut last = Value::Null;
        for i in 0..n {
            let r = if up { n - 1 - i } else { i };
            if let Some(cell) = t.rows.get_mut(r).and_then(|row| row.get_mut(c)) {
                if cell.is_null() {
                    *cell = last.clone();
                } else {
                    last = cell.clone();
                }
            }
        }
    }
    Ok(Value::table(t))
}

fn group(ev: &Evaluator, a: &[Value]) -> Result<Value> {
    let t = table_arg(a, 0, "Table.Group")?;
    let keys = names_arg(arg(a, 1))?;
    let kidx: Vec<usize> = keys.iter().map(|k| col_index(&t, k)).collect::<Result<_>>()?;
    let aggs = pairs(arg(a, 2));
    let local = arg(a, 3).as_number() == Some(0.0);
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    let mut last_key: Option<String> = None;
    let mut local_groups: Vec<Vec<usize>> = Vec::new();
    for r in 0..t.rows.len() {
        let k = key_of(&kidx.iter().map(|c| t.cell(r, *c)).collect::<Vec<_>>());
        if local {
            if last_key.as_ref() != Some(&k) {
                local_groups.push(Vec::new());
                last_key = Some(k);
            }
            if let Some(g) = local_groups.last_mut() {
                g.push(r);
            }
            continue;
        }
        groups.entry(k.clone()).or_insert_with(|| {
            order.push(k.clone());
            Vec::new()
        });
        if let Some(g) = groups.get_mut(&k) {
            g.push(r);
        }
    }
    let group_rows: Vec<Vec<usize>> = if local { local_groups } else { order.iter().filter_map(|k| groups.remove(k)).collect() };
    let mut columns = keys.clone();
    let mut types: Vec<MType> = kidx.iter().map(|c| t.types.get(*c).copied().unwrap_or_default()).collect();
    for p in &aggs {
        columns.push(p.first().and_then(Value::as_text).unwrap_or("Count").to_string());
        types.push(match p.get(2) {
            Some(Value::Type(ty)) => *ty,
            _ => MType::Any,
        });
    }
    let mut rows = Vec::with_capacity(group_rows.len());
    for g in group_rows {
        let sub = Value::table(select_rows(&t, &g));
        let first = g.first().copied().unwrap_or(0);
        let mut row: Vec<Value> = kidx.iter().map(|c| t.cell(first, *c).clone()).collect();
        for p in &aggs {
            let f = p.get(1).cloned().unwrap_or_default();
            row.push(ev.apply1(&f, sub.clone()).unwrap_or_else(|e| Value::error(e.message())));
        }
        rows.push(row);
    }
    Ok(Value::table(Table { columns, types, rows }))
}

fn pivot(ev: &Evaluator, a: &[Value]) -> Result<Value> {
    let t = table_arg(a, 0, "Table.Pivot")?;
    let values: Vec<String> = list_arg(a, 1, "Table.Pivot")?.iter().map(|v| as_text(v).unwrap_or_default()).collect();
    let attr = col_index(&t, &text_arg(a, 2, "Table.Pivot")?)?;
    let val = col_index(&t, &text_arg(a, 3, "Table.Pivot")?)?;
    let agg = arg(a, 4).clone();
    let others: Vec<usize> = (0..t.columns.len()).filter(|c| *c != attr && *c != val).collect();
    let mut order = Vec::new();
    let mut cells: HashMap<String, (usize, HashMap<String, Vec<Value>>)> = HashMap::new();
    for r in 0..t.rows.len() {
        let k = key_of(&others.iter().map(|c| t.cell(r, *c)).collect::<Vec<_>>());
        let entry = cells.entry(k.clone()).or_insert_with(|| {
            order.push(k.clone());
            (r, HashMap::new())
        });
        let name = as_text(t.cell(r, attr)).unwrap_or_default();
        entry.1.entry(name).or_default().push(t.cell(r, val).clone());
    }
    let mut columns: Vec<String> = others.iter().filter_map(|c| t.columns.get(*c).cloned()).collect();
    let mut types: Vec<MType> = others.iter().map(|c| t.types.get(*c).copied().unwrap_or_default()).collect();
    for v in &values {
        columns.push(unique_name(&columns, v));
        types.push(MType::Any);
    }
    let mut rows = Vec::new();
    for k in order {
        let Some((first, map)) = cells.get(&k) else { continue };
        let mut row: Vec<Value> = others.iter().map(|c| t.cell(*first, *c).clone()).collect();
        for v in &values {
            let items = map.get(v).cloned().unwrap_or_default();
            row.push(if agg.is_null() {
                match items.len() {
                    0 => Value::Null,
                    1 => items.into_iter().next().unwrap_or_default(),
                    _ => Value::error("Expression.Error: There were too many elements in the enumeration to complete the operation."),
                }
            } else {
                ev.apply1(&agg, Value::list(items)).unwrap_or_else(|e| Value::error(e.message()))
            });
        }
        rows.push(row);
    }
    Ok(Value::table(Table { columns, types, rows }))
}

fn unpivot(a: &[Value], other: bool) -> Result<Value> {
    let t = table_arg(a, 0, "Table.Unpivot")?;
    let listed = names_arg(arg(a, 1))?;
    let attr = text_arg(a, 2, "Table.Unpivot")?;
    let valname = text_arg(a, 3, "Table.Unpivot")?;
    let cols: Vec<usize> = (0..t.columns.len()).filter(|c| t.columns.get(*c).is_some_and(|n| listed.contains(n) != other)).collect();
    let keep: Vec<usize> = (0..t.columns.len()).filter(|c| !cols.contains(c)).collect();
    let mut columns: Vec<String> = keep.iter().filter_map(|c| t.columns.get(*c).cloned()).collect();
    let mut types: Vec<MType> = keep.iter().map(|c| t.types.get(*c).copied().unwrap_or_default()).collect();
    columns.push(attr);
    columns.push(valname);
    types.push(MType::Text);
    types.push(MType::Any);
    let mut rows = Vec::new();
    for r in 0..t.rows.len() {
        for c in &cols {
            let v = t.cell(r, *c);
            if v.is_null() {
                continue;
            }
            let mut row: Vec<Value> = keep.iter().map(|k| t.cell(r, *k).clone()).collect();
            row.push(Value::text(t.columns.get(*c).cloned().unwrap_or_default()));
            row.push(v.clone());
            rows.push(row);
            if rows.len() > MAX_ROWS {
                return err("The result has too many rows.");
            }
        }
    }
    Ok(Value::table(Table { columns, types, rows }))
}

fn transpose(a: &[Value]) -> Result<Value> {
    let t = table_arg(a, 0, "Table.Transpose")?;
    let n = t.rows.len();
    if n > 16_384 {
        return err("The table has too many rows to transpose.");
    }
    let columns: Vec<String> = match arg(a, 1) {
        Value::List(l) => l.iter().map(|v| as_text(v).unwrap_or_default()).collect(),
        _ => (1..=n).map(|i| format!("Column{i}")).collect(),
    };
    let rows = (0..t.columns.len()).map(|c| (0..n).map(|r| t.cell(r, c).clone()).collect()).collect();
    let k = columns.len();
    Ok(Value::table(Table { columns, types: vec![MType::Any; k], rows }))
}

fn join_keys(t: &Table, v: &Value) -> Result<Vec<usize>> {
    names_arg(v)?.iter().map(|c| col_index(t, c)).collect()
}

/// Rows of `right` by key.
fn index(t: &Table, keys: &[usize]) -> HashMap<String, Vec<usize>> {
    let mut m: HashMap<String, Vec<usize>> = HashMap::new();
    for r in 0..t.rows.len() {
        let vals: Vec<&Value> = keys.iter().map(|c| t.cell(r, *c)).collect();
        if vals.iter().any(|v| v.is_null()) {
            continue;
        }
        m.entry(key_of(&vals)).or_default().push(r);
    }
    m
}

fn nested_join(a: &[Value]) -> Result<Value> {
    let l = table_arg(a, 0, "Table.NestedJoin")?;
    let lk = join_keys(&l, arg(a, 1))?;
    let r = table_arg(a, 2, "Table.NestedJoin")?;
    let rk = join_keys(&r, arg(a, 3))?;
    if lk.len() != rk.len() {
        return err("The key columns must have the same count on both sides.");
    }
    let new = text_arg(a, 4, "Table.NestedJoin")?;
    let kind = opt_num(a, 5).unwrap_or(1.0) as u8;
    let ri = index(&r, &rk);
    let mut columns = l.columns.clone();
    columns.push(unique_name(&l.columns, &new));
    let mut types = l.types.clone();
    types.push(MType::Table);
    let mut rows = Vec::new();
    let mut used = vec![false; r.rows.len()];
    for row in 0..l.rows.len() {
        let key = key_of(&lk.iter().map(|c| l.cell(row, *c)).collect::<Vec<_>>());
        let hits = ri.get(&key).cloned().unwrap_or_default();
        for h in &hits {
            if let Some(u) = used.get_mut(*h) {
                *u = true;
            }
        }
        let include = match kind {
            0 | 2 => !hits.is_empty(),
            4 => hits.is_empty(),
            5 => false,
            _ => true,
        };
        if include {
            let mut out = l.rows.get(row).cloned().unwrap_or_default();
            out.push(Value::table(select_rows(&r, &hits)));
            rows.push(out);
        }
        if rows.len() > MAX_ROWS {
            return err("The result has too many rows.");
        }
    }
    // Right outer, full outer and right anti also keep right rows with no match.
    if matches!(kind, 2 | 3 | 5) {
        for (h, u) in used.iter().enumerate() {
            if !u {
                let mut out = vec![Value::Null; l.columns.len()];
                out.push(Value::table(select_rows(&r, &[h])));
                rows.push(out);
            }
        }
    }
    Ok(Value::table(Table { columns, types, rows }))
}

fn flat_join(a: &[Value]) -> Result<Value> {
    let nested =
        nested_join(&[arg(a, 0).clone(), arg(a, 1).clone(), arg(a, 2).clone(), arg(a, 3).clone(), Value::text("__joined"), arg(a, 4).clone()])?;
    let r = table_arg(a, 2, "Table.Join")?;
    let Value::Table(nt) = &nested else { return err("join failed") };
    let left_cols = nt.columns.len() - 1;
    let new_names: Vec<String> = r.columns.iter().map(|c| unique_name(nt.columns.get(..left_cols).unwrap_or_default(), c)).collect();
    expand_table(&[
        nested.clone(),
        Value::text("__joined"),
        Value::list(r.columns.iter().map(Value::text).collect()),
        Value::list(new_names.iter().map(Value::text).collect()),
    ])
}

/// The table, the expanded column's index, the names to expand and their new names.
type Expansion = (Rc<Table>, usize, Vec<String>, Vec<String>);

fn expand_names(a: &[Value], f: &str) -> Result<Expansion> {
    let t = table_arg(a, 0, f)?;
    let ci = col_index(&t, &text_arg(a, 1, f)?)?;
    let names = names_arg(arg(a, 2))?;
    let new = match arg(a, 3) {
        Value::Null => names.clone(),
        v => names_arg(v)?,
    };
    if new.len() != names.len() {
        return err("The new column names must match the expanded columns.");
    }
    let others: Vec<&String> = t.columns.iter().enumerate().filter(|(i, _)| *i != ci).map(|(_, c)| c).collect();
    if let Some(dup) = new.iter().find(|n| others.contains(n)) {
        return err(format!("The column '{dup}' already exists."));
    }
    Ok((t, ci, names, new))
}

fn spliced(t: &Table, ci: usize, new: &[String]) -> (Vec<String>, Vec<MType>) {
    let mut columns = Vec::new();
    let mut types = Vec::new();
    for (i, c) in t.columns.iter().enumerate() {
        if i == ci {
            columns.extend(new.iter().cloned());
            types.extend(std::iter::repeat_n(MType::Any, new.len()));
        } else {
            columns.push(c.clone());
            types.push(t.types.get(i).copied().unwrap_or_default());
        }
    }
    (columns, types)
}

fn splice_row(row: &[Value], ci: usize, vals: Vec<Value>) -> Vec<Value> {
    let mut out = Vec::with_capacity(row.len() + vals.len());
    out.extend(row.iter().take(ci).cloned());
    out.extend(vals);
    out.extend(row.iter().skip(ci + 1).cloned());
    out
}

fn expand_table(a: &[Value]) -> Result<Value> {
    let (t, ci, names, new) = expand_names(a, "Table.ExpandTableColumn")?;
    let (columns, types) = spliced(&t, ci, &new);
    let mut rows = Vec::new();
    for row in &t.rows {
        match row.get(ci) {
            Some(Value::Table(nt)) if !nt.rows.is_empty() => {
                let idx: Vec<Option<usize>> = names.iter().map(|n| nt.column_index(n)).collect();
                for nr in &nt.rows {
                    rows.push(splice_row(row, ci, idx.iter().map(|i| i.and_then(|i| nr.get(i).cloned()).unwrap_or_default()).collect()));
                    if rows.len() > MAX_ROWS {
                        return err("The result has too many rows.");
                    }
                }
            }
            Some(e @ Value::Error(_)) => rows.push(splice_row(row, ci, vec![e.clone(); names.len()])),
            _ => rows.push(splice_row(row, ci, vec![Value::Null; names.len()])),
        }
    }
    Ok(Value::table(Table { columns, types, rows }))
}

fn expand_record(a: &[Value]) -> Result<Value> {
    let (t, ci, names, new) = expand_names(a, "Table.ExpandRecordColumn")?;
    let (columns, types) = spliced(&t, ci, &new);
    let rows = t
        .rows
        .iter()
        .map(|row| {
            let vals = match row.get(ci) {
                Some(Value::Record(r)) => names.iter().map(|n| r.get(n).cloned().unwrap_or_default()).collect(),
                Some(e @ Value::Error(_)) => vec![e.clone(); names.len()],
                _ => vec![Value::Null; names.len()],
            };
            splice_row(row, ci, vals)
        })
        .collect();
    Ok(Value::table(Table { columns, types, rows }))
}

fn expand_list(a: &[Value]) -> Result<Value> {
    let t = table_arg(a, 0, "Table.ExpandListColumn")?;
    let ci = col_index(&t, &text_arg(a, 1, "Table.ExpandListColumn")?)?;
    let mut rows = Vec::new();
    for row in &t.rows {
        match row.get(ci) {
            Some(Value::List(l)) if !l.is_empty() => {
                for v in l.iter() {
                    rows.push(splice_row(row, ci, vec![v.clone()]));
                    if rows.len() > MAX_ROWS {
                        return err("The result has too many rows.");
                    }
                }
            }
            Some(Value::List(_)) => rows.push(splice_row(row, ci, vec![Value::Null])),
            _ => rows.push(row.clone()),
        }
    }
    let mut types = t.types.clone();
    if let Some(ty) = types.get_mut(ci) {
        *ty = MType::Any;
    }
    Ok(Value::table(Table { columns: t.columns.clone(), types, rows }))
}

fn aggregate_table_column(ev: &Evaluator, a: &[Value]) -> Result<Value> {
    let t = table_arg(a, 0, "Table.AggregateTableColumn")?;
    let ci = col_index(&t, &text_arg(a, 1, "Table.AggregateTableColumn")?)?;
    let specs = pairs(arg(a, 2));
    let new: Vec<String> = specs.iter().map(|p| p.get(2).and_then(Value::as_text).unwrap_or("Aggregate").to_string()).collect();
    let (columns, types) = spliced(&t, ci, &new);
    let mut rows = Vec::new();
    for row in &t.rows {
        let mut vals = Vec::new();
        for p in &specs {
            let col = p.first().and_then(Value::as_text).unwrap_or_default();
            let f = p.get(1).cloned().unwrap_or_default();
            let list = match row.get(ci) {
                Some(Value::Table(nt)) => crate::m::eval::field(&Value::Table(nt.clone()), col, true)?,
                _ => Value::list(vec![]),
            };
            vals.push(ev.apply1(&f, list).unwrap_or_else(|e| Value::error(e.message())));
        }
        rows.push(splice_row(row, ci, vals));
    }
    Ok(Value::table(Table { columns, types, rows }))
}

fn split_column(ev: &Evaluator, a: &[Value]) -> Result<Value> {
    let t = table_arg(a, 0, "Table.SplitColumn")?;
    let src = text_arg(a, 1, "Table.SplitColumn")?;
    let ci = col_index(&t, &src)?;
    let splitter = func_arg(a, 2, "Table.SplitColumn")?;
    let mut parts: Vec<Vec<Value>> = Vec::with_capacity(t.rows.len());
    for r in 0..t.rows.len() {
        let v = t.cell(r, ci);
        parts.push(match v {
            Value::Null => vec![],
            Value::Error(_) => vec![v.clone()],
            other => match check(ev.apply1(&splitter, Value::text(as_text(other).unwrap_or_default()))?)? {
                Value::List(l) => l.to_vec(),
                x => vec![x],
            },
        });
    }
    let new: Vec<String> = match arg(a, 3) {
        Value::Number(n) => (1..=(*n as usize).min(1000)).map(|i| format!("{src}.{i}")).collect(),
        Value::Null => {
            let n = parts.iter().map(Vec::len).max().unwrap_or(1).clamp(1, 1000);
            (1..=n).map(|i| format!("{src}.{i}")).collect()
        }
        v => names_arg(v)?,
    };
    let (columns, types) = spliced(&t, ci, &new);
    let rows = t
        .rows
        .iter()
        .zip(parts)
        .map(|(row, p)| {
            let mut vals: Vec<Value> = p.into_iter().take(new.len()).collect();
            vals.resize(new.len(), arg(a, 4).clone());
            splice_row(row, ci, vals)
        })
        .collect();
    Ok(Value::table(Table { columns, types, rows }))
}

fn combine_columns(ev: &Evaluator, a: &[Value]) -> Result<Value> {
    let t = table_arg(a, 0, "Table.CombineColumns")?;
    let cols = names_arg(arg(a, 1))?;
    let idx: Vec<usize> = cols.iter().map(|c| col_index(&t, c)).collect::<Result<_>>()?;
    let combiner = func_arg(a, 2, "Table.CombineColumns")?;
    let new = text_arg(a, 3, "Table.CombineColumns")?;
    let first = idx.iter().copied().min().unwrap_or(0);
    let mut columns = Vec::new();
    let mut types = Vec::new();
    for (i, c) in t.columns.iter().enumerate() {
        if i == first {
            columns.push(new.clone());
            types.push(MType::Text);
        }
        if !idx.contains(&i) {
            columns.push(c.clone());
            types.push(t.types.get(i).copied().unwrap_or_default());
        }
    }
    let mut rows = Vec::with_capacity(t.rows.len());
    for row in &t.rows {
        let vals: Vec<Value> = idx
            .iter()
            .map(|c| row.get(*c).map(|v| if v.is_null() { Value::Null } else { super::convert::text_from(v) }).unwrap_or_default())
            .collect();
        let combined = ev.apply1(&combiner, Value::list(vals)).unwrap_or_else(|e| Value::error(e.message()));
        let mut out = Vec::new();
        for (i, v) in row.iter().enumerate() {
            if i == first {
                out.push(combined.clone());
            }
            if !idx.contains(&i) {
                out.push(v.clone());
            }
        }
        rows.push(out);
    }
    Ok(Value::table(Table { columns, types, rows }))
}

fn column_names_arg(v: &Value, width: usize) -> Result<Vec<String>> {
    match v {
        Value::Number(n) => Ok((1..=(*n as usize).min(10_000)).map(|i| format!("Column{i}")).collect()),
        Value::Null => Ok((1..=width).map(|i| format!("Column{i}")).collect()),
        Value::Type(_) => Ok((1..=width).map(|i| format!("Column{i}")).collect()),
        v => names_arg(v),
    }
}

fn from_rows(a: &[Value]) -> Result<Value> {
    let rows = list_arg(a, 0, "Table.FromRows")?;
    let width = rows.iter().map(|r| if let Value::List(l) = r { l.len() } else { 0 }).max().unwrap_or(0);
    let columns = column_names_arg(arg(a, 1), width)?;
    let n = columns.len();
    let rows = rows
        .iter()
        .map(|r| {
            let mut v = if let Value::List(l) = r { l.to_vec() } else { vec![] };
            v.resize(n, Value::Null);
            v
        })
        .collect();
    Ok(Value::table(Table { columns, types: vec![MType::Any; n], rows }))
}

fn hash_table(a: &[Value]) -> Result<Value> {
    from_rows(&[arg(a, 1).clone(), arg(a, 0).clone()])
}

fn from_records(a: &[Value]) -> Result<Value> {
    let recs = list_arg(a, 0, "Table.FromRecords")?;
    let mut columns: Vec<String> = Vec::new();
    for r in recs.iter() {
        if let Value::Record(r) = r {
            for n in &r.names {
                if !columns.contains(n) {
                    columns.push(n.clone());
                }
            }
        }
    }
    let rows = recs
        .iter()
        .map(|r| columns.iter().map(|c| if let Value::Record(r) = r { r.get(c).cloned().unwrap_or_default() } else { Value::Null }).collect())
        .collect();
    let n = columns.len();
    Ok(Value::table(Table { columns, types: vec![MType::Any; n], rows }))
}

fn from_columns(a: &[Value]) -> Result<Value> {
    let cols = list_arg(a, 0, "Table.FromColumns")?;
    let lists: Vec<Vec<Value>> = cols.iter().map(|c| if let Value::List(l) = c { l.to_vec() } else { vec![] }).collect();
    let columns = column_names_arg(arg(a, 1), lists.len())?;
    let n = lists.iter().map(Vec::len).max().unwrap_or(0);
    let rows = (0..n).map(|r| (0..columns.len()).map(|c| lists.get(c).and_then(|l| l.get(r)).cloned().unwrap_or_default()).collect()).collect();
    let k = columns.len();
    Ok(Value::table(Table { columns, types: vec![MType::Any; k], rows }))
}

fn from_list(ev: &Evaluator, a: &[Value]) -> Result<Value> {
    let list = list_arg(a, 0, "Table.FromList")?;
    let splitter = arg(a, 1).clone();
    let mut rows = Vec::new();
    for v in list.iter() {
        rows.push(match (&splitter, v) {
            (Value::Null, Value::List(l)) => l.to_vec(),
            (Value::Null, Value::Text(s)) => s.split(',').map(Value::text).collect(),
            (Value::Null, other) => vec![other.clone()],
            (f, other) => match check(ev.apply1(f, other.clone())?)? {
                Value::List(l) => l.to_vec(),
                x => vec![x],
            },
        });
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(1);
    let columns = column_names_arg(arg(a, 2), width)?;
    let n = columns.len();
    for r in &mut rows {
        r.resize(n, arg(a, 3).clone());
    }
    Ok(Value::table(Table { columns, types: vec![MType::Any; n], rows }))
}

/// Splitter and combiner functions (`Splitter.SplitTextByDelimiter(",")`).
pub fn splitter(name: &str, a: &[Value]) -> Option<Result<Value>> {
    let texts = |v: &Value| -> Vec<String> {
        match v {
            Value::Text(t) => vec![t.to_string()],
            Value::List(l) => l.iter().filter_map(|x| x.as_text().map(str::to_string)).collect(),
            _ => vec![],
        }
    };
    let nums = |v: &Value| -> Vec<usize> {
        match v {
            Value::List(l) => l.iter().filter_map(Value::as_number).map(|n| n.max(0.0) as usize).collect(),
            Value::Number(n) => vec![n.max(0.0) as usize],
            _ => vec![],
        }
    };
    let from_end = arg(a, 2).as_number().is_some_and(|x| x != 0.0) || matches!(arg(a, 2), Value::Logical(true));
    let r = match name {
        "Splitter.SplitTextByDelimiter" => {
            let d = arg(a, 0).as_text().unwrap_or(",").to_string();
            let csv = arg(a, 1).as_number() == Some(1.0);
            native(move |_, args| {
                Ok(Value::list(split_text(arg(&args, 0).as_text().unwrap_or_default(), &d, csv).into_iter().map(Value::text).collect()))
            })
        }
        "Splitter.SplitTextByAnyDelimiter" => {
            let ds = texts(arg(a, 0));
            native(move |_, args| {
                let s = arg(&args, 0).as_text().unwrap_or_default().to_string();
                Ok(Value::list(s.split(|c: char| ds.iter().any(|d| d.starts_with(c) && d.chars().count() == 1)).map(Value::text).collect()))
            })
        }
        "Splitter.SplitTextByEachDelimiter" => {
            let ds = texts(arg(a, 0));
            native(move |_, args| {
                let mut s = arg(&args, 0).as_text().unwrap_or_default().to_string();
                let mut out = Vec::new();
                if from_end {
                    let mut tail = Vec::new();
                    for d in ds.iter() {
                        match s.rfind(d.as_str()) {
                            Some(i) => {
                                tail.push(s.get(i + d.len()..).unwrap_or_default().to_string());
                                s.truncate(i);
                            }
                            None => break,
                        }
                    }
                    out.push(s);
                    out.extend(tail.into_iter().rev());
                } else {
                    for d in ds.iter() {
                        match s.find(d.as_str()) {
                            Some(i) => {
                                out.push(s.get(..i).unwrap_or_default().to_string());
                                s = s.get(i + d.len()..).unwrap_or_default().to_string();
                            }
                            None => break,
                        }
                    }
                    out.push(s);
                }
                Ok(Value::list(out.into_iter().map(Value::text).collect()))
            })
        }
        "Splitter.SplitTextByLengths" => {
            let ls = nums(arg(a, 0));
            native(move |_, args| {
                let chars: Vec<char> = arg(&args, 0).as_text().unwrap_or_default().chars().collect();
                let mut pos = 0;
                let mut out = Vec::new();
                for l in &ls {
                    out.push(Value::text(chars.get(pos.min(chars.len())..(pos + l).min(chars.len())).unwrap_or_default().iter().collect::<String>()));
                    pos += l;
                }
                Ok(Value::list(out))
            })
        }
        "Splitter.SplitTextByPositions" => {
            let ps = nums(arg(a, 0));
            native(move |_, args| {
                let chars: Vec<char> = arg(&args, 0).as_text().unwrap_or_default().chars().collect();
                let mut out = Vec::new();
                for (i, p) in ps.iter().enumerate() {
                    let end = ps.get(i + 1).copied().unwrap_or(chars.len()).min(chars.len());
                    out.push(Value::text(chars.get((*p).min(end)..end).unwrap_or_default().iter().collect::<String>()));
                }
                Ok(Value::list(out))
            })
        }
        "Splitter.SplitTextByWhitespace" => {
            native(|_, args| Ok(Value::list(arg(&args, 0).as_text().unwrap_or_default().split_whitespace().map(Value::text).collect())))
        }
        "Splitter.SplitByNothing" => native(|_, args| Ok(Value::list(vec![arg(&args, 0).clone()]))),
        "Splitter.SplitTextByCharacterTransition" => {
            let before = transition_set(arg(a, 0));
            let after = transition_set(arg(a, 1));
            native(move |ev, args| {
                let chars: Vec<char> = arg(&args, 0).as_text().unwrap_or_default().chars().collect();
                let mut out = Vec::new();
                let mut cur = String::new();
                for (i, c) in chars.iter().enumerate() {
                    if i > 0
                        && let Some(p) = chars.get(i - 1)
                        && before.matches(ev, *p)
                        && after.matches(ev, *c)
                    {
                        out.push(Value::text(std::mem::take(&mut cur)));
                    }
                    cur.push(*c);
                }
                out.push(Value::text(cur));
                Ok(Value::list(out))
            })
        }
        "Combiner.CombineTextByDelimiter" => {
            let d = arg(a, 0).as_text().unwrap_or_default().to_string();
            native(move |_, args| {
                let parts = match arg(&args, 0) {
                    Value::List(l) => l.iter().filter(|v| !v.is_null()).map(|v| as_text(v).unwrap_or_default()).collect::<Vec<_>>(),
                    _ => vec![],
                };
                Ok(Value::text(parts.join(&d)))
            })
        }
        "Combiner.CombineTextByLengths" => {
            let ls = nums(arg(a, 0));
            native(move |_, args| {
                let mut s = String::new();
                if let Value::List(l) = arg(&args, 0) {
                    for (v, n) in l.iter().zip(ls.iter()) {
                        let t: String = as_text(v).unwrap_or_default().chars().take(*n).collect();
                        s.push_str(&format!("{t:<n$}", n = *n));
                    }
                }
                Ok(Value::text(s))
            })
        }
        _ => return None,
    };
    Some(Ok(r))
}

enum CharSet {
    Chars(Vec<char>),
    Func(Value),
}

impl CharSet {
    fn matches(&self, ev: &Evaluator, c: char) -> bool {
        match self {
            CharSet::Chars(cs) => cs.contains(&c),
            CharSet::Func(f) => matches!(ev.apply1(f, Value::text(c.to_string())), Ok(Value::Logical(true))),
        }
    }
}

fn transition_set(v: &Value) -> CharSet {
    match v {
        f @ Value::Function(_) => CharSet::Func(f.clone()),
        Value::List(l) => CharSet::Chars(l.iter().filter_map(|x| x.as_text()).flat_map(|s| s.chars().collect::<Vec<_>>()).collect()),
        Value::Text(t) => CharSet::Chars(t.chars().collect()),
        _ => CharSet::Chars(vec![]),
    }
}

/// Splits text by a delimiter; with `csv`, delimiters inside double quotes don't split.
pub fn split_text(s: &str, d: &str, csv: bool) -> Vec<String> {
    if d.is_empty() {
        return vec![s.to_string()];
    }
    if !csv || !s.contains('"') {
        return s.split(d).map(str::to_string).collect();
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        if c == '"' {
            quoted = !quoted;
            cur.push(c);
            rest = rest.get(1..).unwrap_or_default();
            continue;
        }
        if !quoted && rest.starts_with(d) {
            out.push(std::mem::take(&mut cur));
            rest = rest.get(d.len()..).unwrap_or_default();
            continue;
        }
        cur.push(c);
        rest = rest.get(c.len_utf8()..).unwrap_or_default();
    }
    out.push(cur);
    out
}

/// `Replacer.ReplaceText` / `Replacer.ReplaceValue`.
pub fn replacer(name: &str, a: &[Value]) -> Option<Result<Value>> {
    match name {
        "Replacer.ReplaceText" => Some(Ok(match (arg(a, 0), arg(a, 1), arg(a, 2)) {
            (Value::Text(s), Value::Text(old), new) if !old.is_empty() => Value::text(s.replace(&**old, &as_text(new).unwrap_or_default())),
            (v, _, _) => v.clone(),
        })),
        "Replacer.ReplaceValue" => Some(Ok(if values_equal(arg(a, 0), arg(a, 1)) { arg(a, 2).clone() } else { arg(a, 0).clone() })),
        _ => None,
    }
}
