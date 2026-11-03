//! `Date.*`, `DateTime.*`, `Time.*`, `Duration.*` and the `#date` family.

use chrono::{Datelike, Months, NaiveDate, Timelike};

use super::convert::{date_parts, parse_date, parse_datetime, parse_duration, parse_time, to_type};
use super::{arg, err, num_arg, opt_num};
use crate::Result;
use crate::m::eval::Evaluator;
use crate::value::{MType, Value, days_of, serial_of};

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// Keeps the input's kind: a date stays a date, a datetime a datetime.
fn same_kind(v: &Value, d: NaiveDate) -> Value {
    match v {
        Value::DateTime(x) => Value::DateTime(days_of(d) as f64 + x.rem_euclid(1.0)),
        _ => Value::Date(days_of(d)),
    }
}

fn first_day(a: &[Value], i: usize) -> u32 {
    opt_num(a, i).map(|n| n as u32 % 7).unwrap_or(0)
}

/// .NET-style date format codes used by `Date.ToText`: yyyy, yy, MMMM, MMM, MM, M, dddd, ddd,
/// dd, d, HH, hh, mm, ss, tt.
pub fn format_date(dt: chrono::NaiveDateTime, fmt: &str) -> String {
    let mut out = String::new();
    let c: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while let Some(&ch) = c.get(i) {
        let mut n = 1;
        while c.get(i + n) == Some(&ch) {
            n += 1;
        }
        let s = match (ch, n) {
            ('y', 1 | 2) => format!("{:02}", dt.year() % 100),
            ('y', _) => format!("{:04}", dt.year()),
            ('M', 1) => dt.month().to_string(),
            ('M', 2) => format!("{:02}", dt.month()),
            ('M', 3) => MONTHS.get(dt.month0() as usize).map(|m| m.get(..3).unwrap_or(m).to_string()).unwrap_or_default(),
            ('M', _) => MONTHS.get(dt.month0() as usize).copied().unwrap_or_default().to_string(),
            ('d', 1) => dt.day().to_string(),
            ('d', 2) => format!("{:02}", dt.day()),
            ('d', 3) => DAYS.get(dt.weekday().num_days_from_sunday() as usize).map(|d| d.get(..3).unwrap_or(d).to_string()).unwrap_or_default(),
            ('d', _) => DAYS.get(dt.weekday().num_days_from_sunday() as usize).copied().unwrap_or_default().to_string(),
            ('H', 1) => dt.hour().to_string(),
            ('H', _) => format!("{:02}", dt.hour()),
            ('h', 1) => (((dt.hour() + 11) % 12) + 1).to_string(),
            ('h', _) => format!("{:02}", ((dt.hour() + 11) % 12) + 1),
            ('m', 1) => dt.minute().to_string(),
            ('m', _) => format!("{:02}", dt.minute()),
            ('s', 1) => dt.second().to_string(),
            ('s', _) => format!("{:02}", dt.second()),
            ('t', _) => if dt.hour() < 12 { "AM" } else { "PM" }.to_string(),
            (other, k) => std::iter::repeat_n(other, k).collect(),
        };
        out.push_str(&s);
        i += n;
    }
    out
}

pub fn call(_ev: &Evaluator, name: &str, a: &[Value]) -> Option<Result<Value>> {
    let v = arg(a, 0);
    if v.is_null() && !name.starts_with('#') && name != "DateTime.LocalNow" {
        return match name {
            n if n.starts_with("Date.") || n.starts_with("DateTime.") || n.starts_with("Time.") || n.starts_with("Duration.") => {
                Some(Ok(Value::Null))
            }
            _ => None,
        };
    }
    let dt = || date_parts(v).ok_or_else(|| crate::Error::Eval(format!("We cannot convert the value {} to type Date.", v.display())));
    let num = |x: f64| Ok(Value::Number(x));
    let r = match name {
        "#date" => (|| {
            let d = NaiveDate::from_ymd_opt(num_arg(a, 0, name)? as i32, num_arg(a, 1, name)? as u32, num_arg(a, 2, name)? as u32)
                .ok_or_else(|| crate::Error::Eval("The date is not valid.".into()))?;
            Ok(Value::Date(days_of(d)))
        })(),
        "#datetime" => (|| {
            let d = NaiveDate::from_ymd_opt(num_arg(a, 0, name)? as i32, num_arg(a, 1, name)? as u32, num_arg(a, 2, name)? as u32)
                .ok_or_else(|| crate::Error::Eval("The date is not valid.".into()))?;
            let secs = opt_num(a, 3).unwrap_or(0.0) * 3600.0 + opt_num(a, 4).unwrap_or(0.0) * 60.0 + opt_num(a, 5).unwrap_or(0.0);
            Ok(Value::DateTime(days_of(d) as f64 + secs / 86400.0))
        })(),
        "#time" => (|| Ok(Value::Time((num_arg(a, 0, name)? * 3600.0 + num_arg(a, 1, name)? * 60.0 + opt_num(a, 2).unwrap_or(0.0)) / 86400.0)))(),
        "#duration" => (|| {
            Ok(Value::Duration(
                num_arg(a, 0, name)? + (num_arg(a, 1, name)? * 3600.0 + num_arg(a, 2, name)? * 60.0 + opt_num(a, 3).unwrap_or(0.0)) / 86400.0,
            ))
        })(),
        "Date.From" => Ok(to_type(v, MType::Date)),
        "DateTime.From" => Ok(to_type(v, MType::DateTime)),
        "Time.From" => Ok(to_type(v, MType::Time)),
        "Duration.From" => Ok(to_type(v, MType::Duration)),
        "Date.FromText" => Ok(v.as_text().and_then(parse_date).map(|d| Value::Date(days_of(d))).unwrap_or_else(|| {
            Value::error(format!("DataFormat.Error: We couldn't parse the input provided as a Date value. Details: {}", v.display()))
        })),
        "DateTime.FromText" => Ok(v
            .as_text()
            .and_then(parse_datetime)
            .map(|d| Value::DateTime(serial_of(d)))
            .unwrap_or_else(|| Value::error("DataFormat.Error: We couldn't parse the input provided as a DateTime value."))),
        "Time.FromText" => Ok(v
            .as_text()
            .and_then(parse_time)
            .map(|t| Value::Time(t.num_seconds_from_midnight() as f64 / 86400.0))
            .unwrap_or_else(|| Value::error("DataFormat.Error: We couldn't parse the input provided as a Time value."))),
        "Duration.FromText" => Ok(v
            .as_text()
            .and_then(parse_duration)
            .map(Value::Duration)
            .unwrap_or_else(|| Value::error("DataFormat.Error: We couldn't parse the input provided as a Duration value."))),
        "Date.ToText" | "DateTime.ToText" => dt().map(|d| {
            let fmt = match arg(a, 1) {
                Value::Text(f) => f.to_string(),
                Value::Record(r) => r.get("Format").and_then(Value::as_text).unwrap_or("M/d/yyyy").to_string(),
                _ if name == "Date.ToText" => "M/d/yyyy".into(),
                _ => "M/d/yyyy h:mm:ss tt".into(),
            };
            Value::text(format_date(d, &fmt))
        }),
        "Time.ToText" => match v {
            Value::Time(x) => Ok(Value::text(crate::value::time_text(*x))),
            other => Ok(Value::text(other.display())),
        },
        "Duration.ToText" => Ok(Value::text(v.display())),
        "Date.Year" => dt().and_then(|d| num(d.year() as f64)),
        "Date.Month" => dt().and_then(|d| num(d.month() as f64)),
        "Date.Day" => dt().and_then(|d| num(d.day() as f64)),
        "Date.DayOfYear" => dt().and_then(|d| num(d.ordinal() as f64)),
        "Date.QuarterOfYear" => dt().and_then(|d| num((d.month0() / 3 + 1) as f64)),
        "Date.DayOfWeek" => dt().and_then(|d| num(((d.weekday().num_days_from_sunday() + 7 - first_day(a, 1)) % 7) as f64)),
        "Date.DayOfWeekName" => dt().map(|d| Value::text(DAYS.get(d.weekday().num_days_from_sunday() as usize).copied().unwrap_or_default())),
        "Date.MonthName" => dt().map(|d| Value::text(MONTHS.get(d.month0() as usize).copied().unwrap_or_default())),
        "Date.WeekOfYear" => dt().and_then(|d| {
            let jan1 = NaiveDate::from_ymd_opt(d.year(), 1, 1).unwrap_or_default();
            let offset = (jan1.weekday().num_days_from_sunday() + 7 - first_day(a, 1)) % 7;
            num(((d.ordinal0() + offset) / 7 + 1) as f64)
        }),
        "Date.WeekOfMonth" => dt().and_then(|d| {
            let first = NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap_or_default();
            let offset = (first.weekday().num_days_from_sunday() + 7 - first_day(a, 1)) % 7;
            num(((d.day0() + offset) / 7 + 1) as f64)
        }),
        "Date.DaysInMonth" => dt().and_then(|d| {
            let first = NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap_or_default();
            let next = first.checked_add_months(Months::new(1)).unwrap_or(first);
            num((next - first).num_days() as f64)
        }),
        "Date.IsLeapYear" => dt().map(|d| Value::Logical(NaiveDate::from_ymd_opt(d.year(), 2, 29).is_some())),
        "Date.StartOfYear"
        | "Date.StartOfQuarter"
        | "Date.StartOfMonth"
        | "Date.StartOfWeek"
        | "Date.EndOfYear"
        | "Date.EndOfQuarter"
        | "Date.EndOfMonth"
        | "Date.EndOfWeek" => dt().map(|d| {
            let d = d.date();
            let start_m = |m: u32| NaiveDate::from_ymd_opt(d.year(), m, 1).unwrap_or(d);
            let r = match name {
                "Date.StartOfYear" => start_m(1),
                "Date.StartOfQuarter" => start_m(d.month0() / 3 * 3 + 1),
                "Date.StartOfMonth" => start_m(d.month()),
                "Date.StartOfWeek" => d - chrono::Duration::days(((d.weekday().num_days_from_sunday() + 7 - first_day(a, 1)) % 7) as i64),
                "Date.EndOfYear" => NaiveDate::from_ymd_opt(d.year(), 12, 31).unwrap_or(d),
                "Date.EndOfQuarter" => {
                    start_m(d.month0() / 3 * 3 + 1).checked_add_months(Months::new(3)).map(|x| x - chrono::Duration::days(1)).unwrap_or(d)
                }
                "Date.EndOfMonth" => start_m(d.month()).checked_add_months(Months::new(1)).map(|x| x - chrono::Duration::days(1)).unwrap_or(d),
                _ => d + chrono::Duration::days(6 - ((d.weekday().num_days_from_sunday() + 7 - first_day(a, 1)) % 7) as i64),
            };
            same_kind(v, r)
        }),
        "Date.AddDays" | "Date.AddWeeks" => dt().and_then(|d| {
            let n = num_arg(a, 1, name)? * if name == "Date.AddWeeks" { 7.0 } else { 1.0 };
            Ok(match v {
                Value::DateTime(x) => Value::DateTime(x + n),
                _ => Value::Date(days_of(d.date()) + n as i64),
            })
        }),
        "Date.AddMonths" | "Date.AddQuarters" | "Date.AddYears" => dt().and_then(|d| {
            let k = match name {
                "Date.AddMonths" => 1,
                "Date.AddQuarters" => 3,
                _ => 12,
            };
            let n = num_arg(a, 1, name)? as i32 * k;
            let moved =
                if n >= 0 { d.date().checked_add_months(Months::new(n as u32)) } else { d.date().checked_sub_months(Months::new(n.unsigned_abs())) };
            moved.map(|m| same_kind(v, m)).ok_or_else(|| crate::Error::Eval("The date is out of range.".into()))
        }),
        "DateTime.Date" => dt().map(|d| Value::Date(days_of(d.date()))),
        "DateTime.Time" => dt().map(|d| Value::Time(d.time().num_seconds_from_midnight() as f64 / 86400.0)),
        "DateTime.AddZone" => Ok(v.clone()),
        "DateTime.LocalNow" => local_now(),
        "Time.Hour" | "Time.Minute" | "Time.Second" => {
            let secs = match v {
                Value::Time(x) => Some((x * 86400.0).round() as i64),
                Value::DateTime(x) => Some((x.rem_euclid(1.0) * 86400.0).round() as i64),
                _ => None,
            };
            match secs {
                Some(s) => num(match name {
                    "Time.Hour" => s / 3600 % 24,
                    "Time.Minute" => s / 60 % 60,
                    _ => s % 60,
                } as f64),
                None => err("expected a time"),
            }
        }
        "Duration.Days"
        | "Duration.Hours"
        | "Duration.Minutes"
        | "Duration.Seconds"
        | "Duration.TotalDays"
        | "Duration.TotalHours"
        | "Duration.TotalMinutes"
        | "Duration.TotalSeconds" => match v {
            Value::Duration(x) => {
                let secs = (x.fract() * 86400.0).round() as i64;
                num(match name {
                    "Duration.Days" => x.trunc(),
                    "Duration.Hours" => (secs / 3600) as f64,
                    "Duration.Minutes" => (secs / 60 % 60) as f64,
                    "Duration.Seconds" => (secs % 60) as f64,
                    "Duration.TotalDays" => *x,
                    "Duration.TotalHours" => x * 24.0,
                    "Duration.TotalMinutes" => x * 1440.0,
                    _ => x * 86400.0,
                })
            }
            _ => err("expected a duration"),
        },
        _ => return None,
    };
    Some(r)
}

#[cfg(not(target_arch = "wasm32"))]
fn local_now() -> Result<Value> {
    Ok(Value::DateTime(serial_of(chrono::Local::now().naive_local())))
}

#[cfg(target_arch = "wasm32")]
fn local_now() -> Result<Value> {
    err("DateTime.LocalNow isn't available on the web.")
}
