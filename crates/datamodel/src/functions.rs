//! DAX scalar functions whose arguments are all plain values.

use chrono::Datelike;

use crate::value::{Value, compare};
use crate::{Error, Result, dates};

fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Eval(msg.into()))
}

fn v(vals: &[Value], i: usize) -> &Value {
    static BLANK: Value = Value::Blank;
    vals.get(i).unwrap_or(&BLANK)
}

fn n(vals: &[Value], i: usize) -> Result<f64> {
    v(vals, i).to_number()
}

fn want(name: &str, vals: &[Value], min: usize, max: usize) -> Result<()> {
    if vals.len() < min || vals.len() > max {
        return err(format!("{name} takes {} argument(s), not {}", if min == max { min.to_string() } else { format!("{min} to {max}") }, vals.len()));
    }
    Ok(())
}

fn num(x: f64) -> Value {
    Value::Number(x)
}

fn round_to(x: f64, digits: f64, mode: fn(f64) -> f64) -> f64 {
    let f = 10f64.powi(digits.clamp(-15.0, 15.0) as i32);
    mode(x * f) / f
}

pub fn call(name: &str, vals: &[Value]) -> Result<Value> {
    match name {
        "BLANK" => Ok(Value::Blank),
        "TRUE" => Ok(Value::Bool(true)),
        "FALSE" => Ok(Value::Bool(false)),
        "ISBLANK" => Ok(Value::Bool(v(vals, 0).is_blank())),
        "ISNUMBER" => Ok(Value::Bool(matches!(v(vals, 0), Value::Number(_)))),
        "ISTEXT" => Ok(Value::Bool(matches!(v(vals, 0), Value::Text(_)))),
        "AND" => Ok(Value::Bool(v(vals, 0).truthy()? && v(vals, 1).truthy()?)),
        "OR" => Ok(Value::Bool(v(vals, 0).truthy()? || v(vals, 1).truthy()?)),
        "DIVIDE" => divide(vals),
        "MIN" => Ok(if compare(v(vals, 0), v(vals, 1)).is_le() { v(vals, 0).clone() } else { v(vals, 1).clone() }),
        "MAX" => Ok(if compare(v(vals, 0), v(vals, 1)).is_ge() { v(vals, 0).clone() } else { v(vals, 1).clone() }),
        _ => math(name, vals).or_else(|_| text(name, vals)).or_else(|_| date(name, vals)),
    }
}

fn divide(vals: &[Value]) -> Result<Value> {
    want("DIVIDE", vals, 2, 3)?;
    let d = n(vals, 1)?;
    if d == 0.0 {
        return Ok(v(vals, 2).clone());
    }
    if v(vals, 0).is_blank() {
        return Ok(Value::Blank);
    }
    Ok(num(n(vals, 0)? / d))
}

fn math(name: &str, vals: &[Value]) -> Result<Value> {
    let r = match name {
        "ABS" => n(vals, 0)?.abs(),
        "ROUND" => round_to(n(vals, 0)?, n(vals, 1)?, f64::round),
        "ROUNDUP" => round_to(n(vals, 0)?, n(vals, 1)?, |x| if x >= 0.0 { x.ceil() } else { x.floor() }),
        "ROUNDDOWN" | "TRUNC" => round_to(n(vals, 0)?, n(vals, 1)?, f64::trunc),
        "INT" => n(vals, 0)?.floor(),
        "MOD" => {
            let d = n(vals, 1)?;
            if d == 0.0 {
                return err("MOD: division by zero");
            }
            let x = n(vals, 0)?;
            x - d * (x / d).floor()
        }
        "SQRT" => {
            let x = n(vals, 0)?;
            if x < 0.0 {
                return err("SQRT of a negative number");
            }
            x.sqrt()
        }
        "POWER" => n(vals, 0)?.powf(n(vals, 1)?),
        "EXP" => n(vals, 0)?.exp(),
        "LN" => n(vals, 0)?.ln(),
        "LOG10" => n(vals, 0)?.log10(),
        "SIGN" => {
            let x = n(vals, 0)?;
            if x == 0.0 { 0.0 } else { x.signum() }
        }
        "PI" => std::f64::consts::PI,
        "VALUE" => n(vals, 0)?,
        _ => return err(format!("unknown function {name}")),
    };
    Ok(num(r))
}

fn text(name: &str, vals: &[Value]) -> Result<Value> {
    let s = |i: usize| v(vals, i).to_text();
    let count = |i: usize, default: f64| -> Result<usize> {
        let x = if vals.get(i).is_some() { n(vals, i)? } else { default };
        Ok(x.clamp(0.0, 1e7) as usize)
    };
    let r = match name {
        "CONCATENATE" => format!("{}{}", s(0), s(1)),
        "UPPER" => s(0).to_uppercase(),
        "LOWER" => s(0).to_lowercase(),
        "TRIM" => s(0).split_whitespace().collect::<Vec<_>>().join(" "),
        "LEN" => return Ok(num(s(0).chars().count() as f64)),
        "LEFT" => s(0).chars().take(count(1, 1.0)?).collect(),
        "RIGHT" => {
            let t: Vec<char> = s(0).chars().collect();
            let k = count(1, 1.0)?.min(t.len());
            t.get(t.len() - k..).unwrap_or_default().iter().collect()
        }
        "MID" => s(0).chars().skip(count(1, 1.0)?.saturating_sub(1)).take(count(2, 0.0)?).collect(),
        "SUBSTITUTE" => s(0).replace(&s(1), &s(2)),
        "REPT" => {
            let k = count(1, 0.0)?;
            if s(0).len().saturating_mul(k) > 1_000_000 {
                return err("REPT: the result is too long");
            }
            s(0).repeat(k)
        }
        "CONTAINSSTRING" => return Ok(Value::Bool(s(0).to_lowercase().contains(&s(1).to_lowercase()))),
        "FORMAT" => format(v(vals, 0), &s(1)),
        _ => return err(format!("unknown function {name}")),
    };
    Ok(Value::text(r))
}

fn date(name: &str, vals: &[Value]) -> Result<Value> {
    let day = |i: usize| -> Result<chrono::NaiveDate> { dates::to_date(n(vals, i)?).ok_or_else(|| Error::Eval(format!("{name}: not a date"))) };
    let r = match name {
        "DATE" => dates::ymd(n(vals, 0)? as i32, n(vals, 1)?.clamp(1.0, 12.0) as u32, n(vals, 2)?.clamp(1.0, 31.0) as u32)
            .ok_or_else(|| Error::Eval("DATE: not a valid date".into()))?,
        "YEAR" => day(0)?.year() as f64,
        "MONTH" => day(0)?.month() as f64,
        "DAY" => day(0)?.day() as f64,
        "QUARTER" => (day(0)?.month0() / 3 + 1) as f64,
        "WEEKDAY" => (day(0)?.weekday().num_days_from_sunday() + 1) as f64,
        "EOMONTH" => {
            let shifted = dates::shift(n(vals, 0)?, n(vals, 1)? as i32 + 1, "MONTH").and_then(|s| dates::period_start(s, "DATESMTD"));
            shifted.ok_or_else(|| Error::Eval("EOMONTH: not a date".into()))? - 1.0
        }
        _ => return err(format!("unknown function {name}")),
    };
    Ok(num(r))
}

/// FORMAT with the common number patterns: `0`, `0.00`, `#,##0`, `#,##0.00`, `0%`, `0.0%`.
fn format(x: &Value, pattern: &str) -> String {
    let Value::Number(n) = x else { return x.to_text() };
    let pct = pattern.ends_with('%');
    let p = pattern.trim_end_matches('%');
    let decimals = p.split_once('.').map(|(_, d)| d.chars().filter(|c| *c == '0' || *c == '#').count()).unwrap_or(0);
    let val = if pct { n * 100.0 } else { *n };
    let mut s = format!("{val:.decimals$}");
    if p.contains(',') {
        let (int, frac) = s.split_once('.').map(|(a, b)| (a.to_string(), format!(".{b}"))).unwrap_or((s.clone(), String::new()));
        let (sign, digits) = int.strip_prefix('-').map(|d| ("-", d.to_string())).unwrap_or(("", int.clone()));
        let mut grouped = String::new();
        for (i, ch) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(ch);
        }
        s = format!("{sign}{grouped}{frac}");
    }
    if pct { format!("{s}%") } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(format(&Value::Number(1234567.891), "#,##0.00"), "1,234,567.89");
        assert_eq!(format(&Value::Number(0.256), "0.0%"), "25.6%");
        assert_eq!(call("DIVIDE", &[Value::Number(1.0), Value::Number(0.0)]).unwrap(), Value::Blank);
        assert_eq!(call("LEFT", &[Value::text("Hello"), Value::Number(2.0)]).unwrap(), Value::text("He"));
        assert!(call("NOPE", &[]).is_err());
    }
}
