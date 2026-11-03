//! More DAX functions: statistics, percentiles, first/last non-blank, set operations on tables
//! (UNION, INTERSECT, EXCEPT, CROSSJOIN, NATURALINNERJOIN, GENERATE), table constructors (ROW,
//! DATATABLE, CALENDAR, CALENDARAUTO), more time intelligence (DATESINPERIOD, PARALLELPERIOD,
//! NEXT…/STARTOF…/ENDOF…), and functions that need unevaluated arguments (DATEDIFF, ISERROR,
//! ISONORAFTER, EARLIER, CONTAINS, ISINSCOPE, TREATAS, RANK.EQ).

use std::collections::HashSet;
use std::rc::Rc;

use chrono::{Datelike, NaiveDate};

use super::{ColName, Env, Evaluator, Frame, MAX_TABLE_ROWS, TableVal, Val, arg, err, frames, is_ascending};
use crate::dax::Expr;
use crate::filter::{Ctx, Filter};
use crate::value::{Key, Value, compare, sort_order};
use crate::{Error, Result, dates};

/// Variance / standard deviation of `v` (`sample`: divide by n − 1).
pub(super) fn variance(v: &[f64], sample: bool) -> Option<f64> {
    let n = v.len() as f64;
    if v.is_empty() || (sample && v.len() < 2) {
        return None;
    }
    let mean = v.iter().sum::<f64>() / n;
    let ss: f64 = v.iter().map(|x| (x - mean) * (x - mean)).sum();
    Some(ss / if sample { n - 1.0 } else { n })
}

/// The k-th percentile of `v` (inclusive or exclusive method, as Excel and DAX define them).
pub(super) fn percentile(v: &[f64], k: f64, inclusive: bool) -> Result<Value> {
    if v.is_empty() {
        return Ok(Value::Blank);
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len() as f64;
    let rank = if inclusive {
        if !(0.0..=1.0).contains(&k) {
            return err("PERCENTILE.INC: k must be between 0 and 1");
        }
        k * (n - 1.0)
    } else {
        let r = k * (n + 1.0) - 1.0;
        if r < 0.0 || r > n - 1.0 {
            return err("PERCENTILE.EXC: k is outside the range the data allows");
        }
        r
    };
    let lo = rank.floor() as usize;
    let frac = rank - rank.floor();
    let a = s.get(lo).copied().unwrap_or(0.0);
    let b = s.get(lo + 1).copied().unwrap_or(a);
    Ok(Value::Number(a + (b - a) * frac))
}

fn stat(name: &str, v: &[f64]) -> Value {
    let base = name.replace("X.", ".");
    let r = match base.as_str() {
        "STDEV.P" => variance(v, false).map(f64::sqrt),
        "STDEV.S" => variance(v, true).map(f64::sqrt),
        "VAR.P" => variance(v, false),
        "VAR.S" => variance(v, true),
        _ => None,
    };
    r.map(Value::Number).unwrap_or(Value::Blank)
}

/// First and last day of the month, quarter or year holding `serial`.
pub(super) fn period_bounds(serial: f64, unit: &str) -> Option<(f64, f64)> {
    let d = dates::to_date(serial)?;
    let (start, months) = match unit {
        "YEAR" => (NaiveDate::from_ymd_opt(d.year(), 1, 1)?, 12),
        "QUARTER" => (NaiveDate::from_ymd_opt(d.year(), d.month0() / 3 * 3 + 1, 1)?, 3),
        "MONTH" => (NaiveDate::from_ymd_opt(d.year(), d.month(), 1)?, 1),
        "DAY" => return Some((serial.floor(), serial.floor())),
        _ => return None,
    };
    let end = start.checked_add_months(chrono::Months::new(months))?;
    Some((dates::to_serial(start)?, dates::to_serial(end)? - 1.0))
}

/// Now as a serial date-time (local time, or UTC).
pub(super) fn now_serial(utc: bool) -> f64 {
    let (date, secs) = if utc {
        let n = chrono::Utc::now().naive_utc();
        (n.date(), n.time().signed_duration_since(chrono::NaiveTime::MIN).num_seconds())
    } else {
        let n = chrono::Local::now().naive_local();
        (n.date(), n.time().signed_duration_since(chrono::NaiveTime::MIN).num_seconds())
    };
    dates::to_serial(date).unwrap_or(0.0) + secs as f64 / 86400.0
}

fn unit_name(e: &Expr, ev: &Evaluator, ctx: &Ctx, env: &Env) -> Result<String> {
    Ok(match e {
        Expr::Name(u) => u.to_ascii_uppercase(),
        other => ev.scalar(other, ctx, env)?.to_text().to_ascii_uppercase(),
    })
}

/// `DATEDIFF(a, b, unit)`: how many unit boundaries lie between two dates.
pub(super) fn date_diff(a: f64, b: f64, unit: &str) -> Result<f64> {
    let days = b.floor() - a.floor();
    let (Some(da), Some(db)) = (dates::to_date(a), dates::to_date(b)) else { return err("DATEDIFF: not a date") };
    let months = (db.year() - da.year()) as f64 * 12.0 + db.month() as f64 - da.month() as f64;
    Ok(match unit {
        "SECOND" => ((b - a) * 86400.0).round(),
        "MINUTE" => ((b - a) * 1440.0).floor(),
        "HOUR" => ((b - a) * 24.0).floor(),
        "DAY" => days,
        // Week boundaries are Sundays.
        "WEEK" => {
            let wa = (a.floor() - da.weekday().num_days_from_sunday() as f64) / 7.0;
            let wb = (b.floor() - db.weekday().num_days_from_sunday() as f64) / 7.0;
            (wb - wa).round()
        }
        "MONTH" => months,
        "QUARTER" => ((db.year() - da.year()) * 4) as f64 + (db.month0() / 3) as f64 - (da.month0() / 3) as f64,
        "YEAR" => (db.year() - da.year()) as f64,
        other => return err(format!("DATEDIFF: unknown interval {other} (use DAY, WEEK, MONTH, QUARTER, YEAR, HOUR, MINUTE or SECOND)")),
    })
}

impl Evaluator<'_> {
    /// The column names of a table value.
    fn columns_of(&self, tv: &TableVal) -> Vec<ColName> {
        match tv {
            TableVal::Rows { table, .. } => (0..self.m.tables.get(*table).map(|t| t.columns.len()).unwrap_or(0)).map(|c| ColName::Model(*table, c)).collect(),
            TableVal::Values { columns, .. } => columns.clone(),
        }
    }

    /// Every row of a table value as values.
    fn rows_of(&self, tv: &TableVal) -> Vec<Vec<Value>> {
        match tv {
            TableVal::Rows { table, rows } => match self.m.tables.get(*table) {
                Some(t) => rows.iter().map(|r| t.columns.iter().map(|c| c.value(*r as usize).clone()).collect()).collect(),
                None => vec![],
            },
            TableVal::Values { data, .. } => data.clone(),
        }
    }

    fn col_name_text(&self, c: &ColName) -> String {
        match c {
            ColName::Model(t, col) => self.m.tables.get(*t).and_then(|tb| tb.column(*col)).map(|c| c.name.clone()).unwrap_or_default(),
            ColName::Named(n) => n.rsplit_once('[').map(|(_, r)| r.trim_end_matches(']').to_string()).unwrap_or_else(|| n.clone()),
        }
    }

    fn col_numbers(&self, name: &str, args: &[Expr], ctx: &Ctx, env: &Env, with_text: bool) -> Result<Vec<f64>> {
        let (t, c) = self.col_arg(arg(args, 0, name)?, env, name)?;
        let rows = ctx.visible(self.m, t);
        Ok(self
            .column_values(t, c, &rows)
            .filter_map(|v| match v {
                Value::Number(n) => Some(*n),
                Value::Bool(b) if with_text => Some(if *b { 1.0 } else { 0.0 }),
                Value::Text(_) if with_text => Some(0.0),
                _ => None,
            })
            .collect())
    }

    fn x_numbers(&self, name: &str, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<Vec<f64>> {
        let tv = self.table(arg(args, 0, name)?, ctx, env)?;
        let body = arg(args, 1, name)?;
        let mut out = Vec::with_capacity(tv.len());
        for f in frames(&tv) {
            if let Some(x) = self.scalar(body, ctx, &env.with_frame(f))?.as_number() {
                out.push(x);
            }
        }
        Ok(out)
    }

    /// Distinct visible dates of a date column, sorted, and every date the column holds.
    #[allow(clippy::type_complexity)]
    fn date_sets(&self, name: &str, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<((usize, usize), Vec<f64>, Vec<f64>)> {
        let (t, c) = self.col_arg(arg(args, 0, name)?, env, name)?;
        let Some(col) = self.m.tables.get(t).and_then(|tb| tb.column(c)) else { return err("no such column") };
        let mut all: Vec<f64> = (0..col.cardinality() as u32).filter_map(|id| col.dict_value(id).as_number()).collect();
        all.sort_by(f64::total_cmp);
        let rows = ctx.visible(self.m, t);
        let mut cur: Vec<f64> = self.column_values(t, c, &rows).filter_map(Value::as_number).collect();
        cur.sort_by(f64::total_cmp);
        cur.dedup();
        Ok(((t, c), cur, all))
    }

    fn date_table(&self, (t, c): (usize, usize), picked: impl IntoIterator<Item = f64>) -> TableVal {
        let mut p: Vec<f64> = picked.into_iter().collect();
        p.sort_by(f64::total_cmp);
        p.dedup();
        TableVal::Values { columns: vec![ColName::Model(t, c)], data: p.into_iter().map(|d| vec![Value::Number(d)]).collect() }
    }

    /// Functions handled here; `None` when `name` isn't one of them.
    pub(super) fn call_ext(&self, name: &str, args: &[Expr], ctx: &Ctx, env: &Env) -> Option<Result<Val>> {
        let s = |v: Result<Value>| Some(v.map(Val::Scalar));
        let t = |v: Result<TableVal>| Some(v.map(|t| Val::Table(Rc::new(t))));
        match name {
            "STDEV.P" | "STDEV.S" | "VAR.P" | "VAR.S" => s(self.col_numbers(name, args, ctx, env, false).map(|v| stat(name, &v))),
            "STDEVX.P" | "STDEVX.S" | "VARX.P" | "VARX.S" => s(self.x_numbers(name, args, ctx, env).map(|v| stat(name, &v))),
            "PERCENTILE.INC" | "PERCENTILE.EXC" => s((|| {
                let v = self.col_numbers(name, args, ctx, env, false)?;
                let k = self.scalar(arg(args, 1, name)?, ctx, env)?.to_number()?;
                percentile(&v, k, name.ends_with("INC"))
            })()),
            "PERCENTILEX.INC" | "PERCENTILEX.EXC" => s((|| {
                let v = self.x_numbers(name, args, ctx, env)?;
                let k = self.scalar(arg(args, 2, name)?, ctx, env)?.to_number()?;
                percentile(&v, k, name.ends_with("INC"))
            })()),
            "MAXA" | "MINA" | "AVERAGEA" => s(self.col_numbers(name, args, ctx, env, true).map(|v| {
                if v.is_empty() {
                    return Value::Blank;
                }
                Value::Number(match name {
                    "MAXA" => v.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    "MINA" => v.iter().copied().fold(f64::INFINITY, f64::min),
                    _ => v.iter().sum::<f64>() / v.len() as f64,
                })
            })),
            "APPROXIMATEDISTINCTCOUNT" => Some(self.call("DISTINCTCOUNT", args, ctx, env)),
            "FIRSTNONBLANK" | "LASTNONBLANK" | "FIRSTNONBLANKVALUE" | "LASTNONBLANKVALUE" => Some(self.non_blank(name, args, ctx, env)),
            "CONTAINS" => s(self.contains(args, ctx, env)),
            "ISINSCOPE" | "HASONEFILTER" => s((|| {
                let (t, c) = self.col_arg(arg(args, 0, name)?, env, name)?;
                let one = ctx.filters.iter().any(|f| matches!(f, Filter::Values { table, column, keys } if *table == t && *column == c && keys.len() == 1));
                Ok(Value::Bool(one))
            })()),
            "DATEDIFF" => s((|| {
                let a = self.scalar(arg(args, 0, name)?, ctx, env)?;
                let b = self.scalar(arg(args, 1, name)?, ctx, env)?;
                if a.is_blank() || b.is_blank() {
                    return Ok(Value::Blank);
                }
                let unit = unit_name(arg(args, 2, name)?, self, ctx, env)?;
                Ok(Value::Number(date_diff(a.to_number()?, b.to_number()?, &unit)?))
            })()),
            "ISERROR" => s(Ok(Value::Bool(match self.scalar(arg(args, 0, name).ok()?, ctx, env) {
                Ok(v) => v.as_number().is_some_and(|x| !x.is_finite()),
                Err(_) => true,
            }))),
            "ISONORAFTER" => s(self.is_on_or_after(args, ctx, env)),
            "EARLIER" | "EARLIEST" => s(self.earlier(name, args, ctx, env)),
            "RANK.EQ" => s(self.rank_eq(args, ctx, env)),
            "TREATAS" => t(self.treat_as(args, ctx, env)),
            "CROSSFILTER" => s(Ok(Value::Blank)),
            "ALLSELECTED" | "ALLCROSSFILTERED" => Some(if args.is_empty() { err("ALLSELECTED() with no arguments can only be a CALCULATE filter") } else { self.call("ALL", args, ctx, env) }),
            "FILTERS" => Some(self.call("VALUES", args, ctx, env)),
            "KEEPFILTERS" if args.len() == 1 => Some(self.eval(arg(args, 0, name).ok()?, ctx, env)),
            "UNION" => t(self.union(args, ctx, env)),
            "INTERSECT" | "EXCEPT" => t(self.intersect_except(name, args, ctx, env)),
            "CROSSJOIN" => t(self.crossjoin(args, ctx, env)),
            "NATURALINNERJOIN" | "NATURALLEFTOUTERJOIN" => t(self.natural_join(name, args, ctx, env)),
            "GENERATE" | "GENERATEALL" => t(self.generate(name, args, ctx, env)),
            "ROW" => t(self.row_fn(args, ctx, env)),
            "DATATABLE" => t(self.datatable(args, ctx, env)),
            "CALENDAR" => t((|| {
                let a = self.scalar(arg(args, 0, name)?, ctx, env)?.to_number()?.floor();
                let b = self.scalar(arg(args, 1, name)?, ctx, env)?.to_number()?.floor();
                calendar(a, b)
            })()),
            "CALENDARAUTO" => t(self.calendar_auto(args, ctx, env)),
            "SAMPLE" => t(self.sample(args, ctx, env)),
            "DATESINPERIOD" => t((|| {
                let (tc, _, all) = self.date_sets(name, args, ctx, env)?;
                let start = self.scalar(arg(args, 1, name)?, ctx, env)?.to_number()?.floor();
                let n = self.scalar(arg(args, 2, name)?, ctx, env)?.to_number()? as i32;
                let unit = unit_name(arg(args, 3, name)?, self, ctx, env)?;
                let other = dates::shift(start, n, &unit).ok_or_else(|| Error::Eval("DATESINPERIOD: not a date".into()))?;
                let (lo, hi) = if n >= 0 { (start, other - 1.0) } else { (other + 1.0, start) };
                Ok(self.date_table(tc, all.into_iter().filter(|d| *d >= lo && *d <= hi)))
            })()),
            "PARALLELPERIOD" => t((|| {
                let (tc, cur, all) = self.date_sets(name, args, ctx, env)?;
                let (Some(&lo), Some(&hi)) = (cur.first(), cur.last()) else { return Ok(self.date_table(tc, [])) };
                let n = self.scalar(arg(args, 1, name)?, ctx, env)?.to_number()? as i32;
                let unit = unit_name(arg(args, 2, name)?, self, ctx, env)?;
                let a = dates::shift(lo, n, &unit).and_then(|d| period_bounds(d, &unit)).map(|b| b.0);
                let b = dates::shift(hi, n, &unit).and_then(|d| period_bounds(d, &unit)).map(|b| b.1);
                let (Some(a), Some(b)) = (a, b) else { return err("PARALLELPERIOD: use MONTH, QUARTER or YEAR") };
                Ok(self.date_table(tc, all.into_iter().filter(|d| *d >= a && *d <= b)))
            })()),
            "NEXTDAY" | "NEXTMONTH" | "NEXTQUARTER" | "NEXTYEAR" | "PREVIOUSDAY" => t((|| {
                let (tc, cur, all) = self.date_sets(name, args, ctx, env)?;
                let (Some(&lo), Some(&hi)) = (cur.first(), cur.last()) else { return Ok(self.date_table(tc, [])) };
                let (from, unit, n) = match name {
                    "PREVIOUSDAY" => (lo, "DAY", -1),
                    "NEXTDAY" => (hi, "DAY", 1),
                    "NEXTMONTH" => (hi, "MONTH", 1),
                    "NEXTQUARTER" => (hi, "QUARTER", 1),
                    _ => (hi, "YEAR", 1),
                };
                let (a, b) = dates::shift(from, n, unit).and_then(|d| period_bounds(d, unit)).ok_or_else(|| Error::Eval(format!("{name}: not a date")))?;
                Ok(self.date_table(tc, all.into_iter().filter(|d| *d >= a && *d <= b)))
            })()),
            "FIRSTDATE" | "LASTDATE" | "STARTOFMONTH" | "STARTOFQUARTER" | "STARTOFYEAR" | "ENDOFMONTH" | "ENDOFQUARTER" | "ENDOFYEAR" => t((|| {
                let (tc, cur, all) = self.date_sets(name, args, ctx, env)?;
                let (Some(&lo), Some(&hi)) = (cur.first(), cur.last()) else { return Ok(self.date_table(tc, [])) };
                let pick = match name {
                    "FIRSTDATE" => Some(lo),
                    "LASTDATE" => Some(hi),
                    _ => {
                        let unit = if name.ends_with("MONTH") {
                            "MONTH"
                        } else if name.ends_with("QUARTER") {
                            "QUARTER"
                        } else {
                            "YEAR"
                        };
                        if name.starts_with("START") {
                            let (a, _) = period_bounds(lo, unit).ok_or_else(|| Error::Eval("not a date".into()))?;
                            all.iter().copied().find(|d| *d >= a)
                        } else {
                            let (_, b) = period_bounds(hi, unit).ok_or_else(|| Error::Eval("not a date".into()))?;
                            all.iter().rev().copied().find(|d| *d <= b)
                        }
                    }
                };
                Ok(self.date_table(tc, pick))
            })()),
            "TODAY" | "UTCTODAY" => s(Ok(Value::Number(now_serial(name == "UTCTODAY").floor()))),
            "NOW" | "UTCNOW" => s(Ok(Value::Number(now_serial(name == "UTCNOW")))),
            "USERNAME" | "USERPRINCIPALNAME" | "USEROBJECTID" => s(Ok(Value::text("user@vizcraft.local"))),
            "SELECTEDMEASURE" | "SELECTEDMEASURENAME" => s(err(format!("{name} is only available in calculation groups, which VizCraft doesn't have"))),
            _ => None,
        }
    }

    fn non_blank(&self, name: &str, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<Val> {
        let (t, c) = self.col_arg(arg(args, 0, name)?, env, name)?;
        let body = arg(args, 1, name)?;
        let rows = ctx.visible(self.m, t);
        let mut vals = self.distinct_tuples(t, &[c], rows.iter())?;
        if name.starts_with("LAST") {
            vals.reverse();
        }
        let columns = Rc::new(vec![ColName::Model(t, c)]);
        for tup in vals {
            let f = Frame::Tuple { columns: columns.clone(), values: Rc::new(tup.clone()) };
            // The expression is evaluated with the value as a filter (as CALCULATE would).
            let mut c2 = ctx.clone();
            if let Some(v) = tup.first() {
                c2.replace(Filter::Values { table: t, column: c, keys: Rc::new(HashSet::from([v.key()])) });
            }
            let r = self.scalar(body, &c2, &env.with_frame(f).without_rows())?;
            if !r.is_blank() {
                return Ok(if name.ends_with("VALUE") {
                    Val::Scalar(r)
                } else {
                    Val::Table(Rc::new(TableVal::Values { columns: vec![ColName::Model(t, c)], data: vec![tup] }))
                });
            }
        }
        Ok(if name.ends_with("VALUE") { Val::Scalar(Value::Blank) } else { Val::Table(Rc::new(TableVal::Values { columns: vec![ColName::Model(t, c)], data: vec![] })) })
    }

    fn contains(&self, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<Value> {
        let tv = self.table(arg(args, 0, "CONTAINS")?, ctx, env)?;
        let mut conds = Vec::new();
        let mut i = 1;
        while let (Some(col), Some(val)) = (args.get(i), args.get(i + 1)) {
            conds.push((col, self.scalar(val, ctx, env)?));
            i += 2;
        }
        if conds.is_empty() {
            return err("CONTAINS needs pairs of column, value");
        }
        for f in frames(&tv) {
            let env2 = env.with_frame(f);
            let mut all = true;
            for (col, v) in &conds {
                if !compare(&self.scalar(col, ctx, &env2)?, v).is_eq() {
                    all = false;
                    break;
                }
            }
            if all {
                return Ok(Value::Bool(true));
            }
        }
        Ok(Value::Bool(false))
    }

    fn is_on_or_after(&self, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<Value> {
        let mut i = 0;
        while let (Some(a), Some(b)) = (args.get(i), args.get(i + 1)) {
            let (desc, step) = match args.get(i + 2) {
                Some(Expr::Name(o)) if o.eq_ignore_ascii_case("DESC") => (true, 3),
                Some(Expr::Name(o)) if o.eq_ignore_ascii_case("ASC") => (false, 3),
                _ => (false, 2),
            };
            let o = compare(&self.scalar(a, ctx, env)?, &self.scalar(b, ctx, env)?);
            if o.is_ne() {
                return Ok(Value::Bool(if desc { o.is_lt() } else { o.is_gt() }));
            }
            i += step;
        }
        Ok(Value::Bool(true))
    }

    fn earlier(&self, name: &str, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<Value> {
        let (t, c) = self.col_arg(arg(args, 0, name)?, env, name)?;
        let n = match args.get(1) {
            Some(e) => self.scalar(e, ctx, env)?.to_number()?.max(1.0) as usize,
            None => 1,
        };
        let matching: Vec<&Frame> = env
            .frames
            .iter()
            .filter(|f| match f {
                Frame::Row { table, .. } => *table == t,
                Frame::Tuple { columns, .. } => columns.contains(&ColName::Model(t, c)),
            })
            .collect();
        let f = if name == "EARLIEST" { matching.first() } else { matching.len().checked_sub(n + 1).and_then(|i| matching.get(i)) };
        match f {
            Some(Frame::Row { row, .. }) => Ok(self.m.tables.get(t).and_then(|tb| tb.column(c)).map(|col| col.value(*row as usize).clone()).unwrap_or_default()),
            Some(Frame::Tuple { columns, values }) => {
                Ok(columns.iter().position(|cn| *cn == ColName::Model(t, c)).and_then(|i| values.get(i).cloned()).unwrap_or_default())
            }
            None => err(format!("{name}: there is no outer row context for {}", self.m.column_name(t, c))),
        }
    }

    fn rank_eq(&self, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<Value> {
        let v = self.scalar(arg(args, 0, "RANK.EQ")?, ctx, env)?;
        if v.is_blank() {
            return Ok(Value::Blank);
        }
        let (t, c) = self.col_arg(arg(args, 1, "RANK.EQ")?, env, "RANK.EQ")?;
        let asc = match args.get(2) {
            Some(o) => super::is_ascending(o) || matches!(self.scalar(o, ctx, env), Ok(Value::Number(n)) if n != 0.0),
            None => false,
        };
        let all = crate::store::RowSet::all(self.m.tables.get(t).map(|tb| tb.row_count()).unwrap_or(0));
        let ahead = self.column_values(t, c, &all).filter(|x| !x.is_blank()).filter(|x| if asc { compare(x, &v).is_lt() } else { compare(x, &v).is_gt() }).count();
        Ok(Value::Number(ahead as f64 + 1.0))
    }

    fn treat_as(&self, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<TableVal> {
        let tv = self.table(arg(args, 0, "TREATAS")?, ctx, env)?;
        let mut cols = Vec::new();
        for a in args.get(1..).unwrap_or_default() {
            cols.push(self.col_arg(a, env, "TREATAS")?);
        }
        if cols.is_empty() {
            return err("TREATAS needs the columns to apply the values to");
        }
        let data: Vec<Vec<Value>> = self.rows_of(&tv).into_iter().map(|r| r.into_iter().take(cols.len()).collect()).collect();
        Ok(TableVal::Values { columns: cols.iter().map(|(t, c)| ColName::Model(*t, *c)).collect(), data })
    }

    fn union(&self, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<TableVal> {
        let mut tables = Vec::new();
        for a in args {
            tables.push(self.table(a, ctx, env)?);
        }
        let Some(first) = tables.first() else { return err("UNION needs at least two tables") };
        // Rows of one model table stay rows of it.
        if let TableVal::Rows { table, .. } = &**first
            && tables.iter().all(|t| matches!(&**t, TableVal::Rows { table: tt, .. } if tt == table))
        {
            let rows = tables.iter().flat_map(|t| if let TableVal::Rows { rows, .. } = &**t { rows.clone() } else { vec![] }).collect();
            return Ok(TableVal::Rows { table: *table, rows });
        }
        let columns = self.columns_of(first);
        let width = columns.len();
        let mut data = Vec::new();
        for t in &tables {
            if self.columns_of(t).len() != width {
                return err("UNION: every table must have the same number of columns");
            }
            data.extend(self.rows_of(t));
            if data.len() > MAX_TABLE_ROWS {
                return err("the result has too many rows");
            }
        }
        Ok(TableVal::Values { columns, data })
    }

    fn intersect_except(&self, name: &str, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<TableVal> {
        let a = self.table(arg(args, 0, name)?, ctx, env)?;
        let b = self.table(arg(args, 1, name)?, ctx, env)?;
        if self.columns_of(&a).len() != self.columns_of(&b).len() {
            return err(format!("{name}: both tables must have the same number of columns"));
        }
        let keys: HashSet<Vec<Key>> = self.rows_of(&b).iter().map(|r| r.iter().map(Value::key).collect()).collect();
        let want = name == "INTERSECT";
        let keep: Vec<usize> =
            self.rows_of(&a).iter().enumerate().filter(|(_, r)| keys.contains(&r.iter().map(Value::key).collect::<Vec<_>>()) == want).map(|(i, _)| i).collect();
        Ok(a.select(&keep))
    }

    fn crossjoin(&self, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<TableVal> {
        let mut columns: Vec<ColName> = Vec::new();
        let mut data: Vec<Vec<Value>> = vec![vec![]];
        for a in args {
            let tv = self.table(a, ctx, env)?;
            columns.extend(self.columns_of(&tv));
            let rows = self.rows_of(&tv);
            if data.len().saturating_mul(rows.len()) > MAX_TABLE_ROWS {
                return err("the result has too many rows");
            }
            let mut next = Vec::with_capacity(data.len() * rows.len());
            for d in &data {
                for r in &rows {
                    let mut x = d.clone();
                    x.extend(r.iter().cloned());
                    next.push(x);
                }
            }
            data = next;
        }
        Ok(TableVal::Values { columns, data })
    }

    fn natural_join(&self, name: &str, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<TableVal> {
        let a = self.table(arg(args, 0, name)?, ctx, env)?;
        let b = self.table(arg(args, 1, name)?, ctx, env)?;
        let (ca, cb) = (self.columns_of(&a), self.columns_of(&b));
        let na: Vec<String> = ca.iter().map(|c| self.col_name_text(c).to_lowercase()).collect();
        let nb: Vec<String> = cb.iter().map(|c| self.col_name_text(c).to_lowercase()).collect();
        let common: Vec<(usize, usize)> = na.iter().enumerate().filter_map(|(i, n)| nb.iter().position(|m| m == n).map(|j| (i, j))).collect();
        if common.is_empty() {
            return err(format!("{name}: the tables have no column names in common"));
        }
        let extra: Vec<usize> = (0..cb.len()).filter(|j| !common.iter().any(|(_, cj)| cj == j)).collect();
        let mut columns = ca.clone();
        columns.extend(extra.iter().filter_map(|j| cb.get(*j).cloned()));
        let rows_b = self.rows_of(&b);
        let mut data = Vec::new();
        for ra in self.rows_of(&a) {
            let mut matched = false;
            for rb in &rows_b {
                if common.iter().all(|(i, j)| ra.get(*i).map(Value::key) == rb.get(*j).map(Value::key)) {
                    matched = true;
                    let mut row = ra.clone();
                    row.extend(extra.iter().map(|j| rb.get(*j).cloned().unwrap_or_default()));
                    data.push(row);
                    if data.len() > MAX_TABLE_ROWS {
                        return err("the result has too many rows");
                    }
                }
            }
            if !matched && name == "NATURALLEFTOUTERJOIN" {
                let mut row = ra.clone();
                row.extend(extra.iter().map(|_| Value::Blank));
                data.push(row);
            }
        }
        Ok(TableVal::Values { columns, data })
    }

    fn generate(&self, name: &str, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<TableVal> {
        let a = self.table(arg(args, 0, name)?, ctx, env)?;
        let body = arg(args, 1, name)?;
        let mut columns = self.columns_of(&a);
        let mut extra_cols: Option<Vec<ColName>> = None;
        let mut data = Vec::new();
        let rows_a = self.rows_of(&a);
        let mut pending_outer = Vec::new();
        for (f, ra) in frames(&a).into_iter().zip(rows_a) {
            let b = self.table(body, ctx, &env.with_frame(f))?;
            if extra_cols.is_none() && !b.is_empty() {
                extra_cols = Some(self.columns_of(&b));
            }
            let rows_b = self.rows_of(&b);
            if rows_b.is_empty() && name == "GENERATEALL" {
                pending_outer.push(data.len());
                data.push(ra.clone());
            }
            for rb in rows_b {
                let mut row = ra.clone();
                row.extend(rb);
                data.push(row);
                if data.len() > MAX_TABLE_ROWS {
                    return err("the result has too many rows");
                }
            }
        }
        let extra = extra_cols.unwrap_or_default();
        for i in pending_outer {
            if let Some(r) = data.get_mut(i) {
                r.resize(columns.len() + extra.len(), Value::Blank);
            }
        }
        columns.extend(extra);
        Ok(TableVal::Values { columns, data })
    }

    fn row_fn(&self, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<TableVal> {
        let mut columns = Vec::new();
        let mut row = Vec::new();
        let mut i = 0;
        while let Some(a) = args.get(i) {
            let Expr::Text(n) = a else { return err("ROW takes pairs of \"Name\", expression") };
            columns.push(ColName::Named(n.clone()));
            row.push(self.scalar(arg(args, i + 1, "ROW")?, ctx, env)?);
            i += 2;
        }
        Ok(TableVal::Values { columns, data: vec![row] })
    }

    fn datatable(&self, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<TableVal> {
        let Some((Expr::TableCtor(rows), defs)) = args.split_last() else { return err("DATATABLE ends with the rows: {{…}, {…}}") };
        let mut columns = Vec::new();
        let mut types = Vec::new();
        for pair in defs.chunks(2) {
            match pair {
                [Expr::Text(n), Expr::Name(ty)] => {
                    columns.push(ColName::Named(n.clone()));
                    types.push(ty.to_ascii_uppercase());
                }
                _ => return err("DATATABLE takes pairs of \"Column\", TYPE (STRING, INTEGER, DOUBLE, CURRENCY, BOOLEAN, DATETIME)"),
            }
        }
        let mut data = Vec::new();
        for r in rows {
            // `{1, "a"}` inside the outer braces is a one-column table constructor.
            let cells: Vec<&Expr> = match r.as_slice() {
                [Expr::TableCtor(inner)] => inner.iter().flatten().collect(),
                other => other.iter().collect(),
            };
            let mut out = Vec::with_capacity(columns.len());
            for (i, e) in cells.iter().enumerate() {
                let v = self.scalar(e, ctx, env)?;
                let ty = types.get(i).map(String::as_str).unwrap_or("STRING");
                out.push(match (ty, &v) {
                    (_, Value::Blank) => Value::Blank,
                    ("STRING", _) => Value::text(v.to_text()),
                    ("BOOLEAN", _) => Value::Bool(v.truthy()?),
                    ("DATETIME", Value::Text(s)) => parse_iso_date(s).map(Value::Number).unwrap_or(Value::Blank),
                    ("INTEGER", _) => Value::Number(v.to_number()?.round()),
                    _ => Value::Number(v.to_number()?),
                });
            }
            out.resize(columns.len(), Value::Blank);
            data.push(out);
        }
        Ok(TableVal::Values { columns, data })
    }

    fn calendar_auto(&self, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<TableVal> {
        let fy_end = match args.first() {
            Some(e) => self.scalar(e, ctx, env)?.to_number()?.clamp(1.0, 12.0) as u32,
            None => 12,
        };
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for t in &self.m.tables {
            for c in &t.columns {
                if !c.name.to_lowercase().contains("date") || c.name.contains('.') {
                    continue;
                }
                for id in 0..c.cardinality() as u32 {
                    if let Some(x) = c.dict_value(id).as_number()
                        && (2.0..2_958_465.0).contains(&x)
                    {
                        lo = lo.min(x);
                        hi = hi.max(x);
                    }
                }
            }
        }
        if !lo.is_finite() {
            return err("CALENDARAUTO: the model has no date columns");
        }
        let (Some(a), Some(b)) = (dates::to_date(lo), dates::to_date(hi)) else { return err("CALENDARAUTO: not a date") };
        // Whole fiscal years ending in month `fy_end`.
        let start_month = fy_end % 12 + 1;
        let start_year = if start_month == 1 || a.month() >= start_month { a.year() } else { a.year() - 1 };
        let start = dates::ymd(start_year, start_month, 1).ok_or_else(|| Error::Eval("CALENDARAUTO: not a date".into()))?;
        let end_year = if b.month() > fy_end { b.year() + 1 } else { b.year() };
        let end = period_bounds(dates::ymd(end_year, fy_end, 1).unwrap_or(hi), "MONTH").map(|p| p.1).unwrap_or(hi);
        calendar(start, end)
    }

    fn sample(&self, args: &[Expr], ctx: &Ctx, env: &Env) -> Result<TableVal> {
        let n = self.scalar(arg(args, 0, "SAMPLE")?, ctx, env)?.to_number()?.max(0.0) as usize;
        let tv = self.table(arg(args, 1, "SAMPLE")?, ctx, env)?;
        let mut keyed = Vec::with_capacity(tv.len());
        for (i, f) in frames(&tv).into_iter().enumerate() {
            let env2 = env.with_frame(f);
            let mut keys = Vec::new();
            let mut j = 2;
            while let Some(e) = args.get(j) {
                let asc = args.get(j + 1).map(is_ascending).unwrap_or(true);
                keys.push((self.scalar(e, ctx, &env2)?, asc));
                j += 2;
            }
            keyed.push((keys, i));
        }
        keyed.sort_by(|a, b| {
            a.0.iter().zip(b.0.iter()).map(|((x, asc), (y, _))| if *asc { sort_order(x, y) } else { sort_order(y, x) }).find(|o| o.is_ne()).unwrap_or(std::cmp::Ordering::Equal)
        });
        let len = keyed.len();
        if n == 0 || len == 0 {
            return Ok(tv.select(&[]));
        }
        let keep: Vec<usize> = if n >= len {
            keyed.iter().map(|(_, i)| *i).collect()
        } else {
            (0..n).filter_map(|k| keyed.get(((k as f64 + 0.5) * len as f64 / n as f64) as usize).map(|(_, i)| *i)).collect()
        };
        Ok(tv.select(&keep))
    }
}

fn calendar(a: f64, b: f64) -> Result<TableVal> {
    if b < a {
        return err("CALENDAR: the end date is before the start date");
    }
    if b - a > MAX_TABLE_ROWS as f64 {
        return err("the result has too many rows");
    }
    let data = (a as i64..=b as i64).map(|d| vec![Value::Number(d as f64)]).collect();
    Ok(TableVal::Values { columns: vec![ColName::Named("Date".into())], data })
}

fn parse_iso_date(s: &str) -> Option<f64> {
    let s = s.trim();
    let d = NaiveDate::parse_from_str(s.get(..10).unwrap_or(s), "%Y-%m-%d").or_else(|_| NaiveDate::parse_from_str(s, "%m/%d/%Y")).ok()?;
    let mut serial = dates::to_serial(d)?;
    if let Some(time) = s.get(11..).and_then(|t| chrono::NaiveTime::parse_from_str(t, "%H:%M:%S").ok()) {
        serial += time.signed_duration_since(chrono::NaiveTime::MIN).num_seconds() as f64 / 86400.0;
    }
    Some(serial)
}
