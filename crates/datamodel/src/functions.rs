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
        "ISLOGICAL" => Ok(Value::Bool(matches!(v(vals, 0), Value::Bool(_)))),
        "ISNONTEXT" => Ok(Value::Bool(!matches!(v(vals, 0), Value::Text(_)))),
        "ISEVEN" => Ok(Value::Bool((n(vals, 0)?.trunc() as i64) % 2 == 0)),
        "ISODD" => Ok(Value::Bool((n(vals, 0)?.trunc() as i64) % 2 != 0)),
        "IF.EAGER" => Ok(if v(vals, 0).truthy()? { v(vals, 1).clone() } else { v(vals, 2).clone() }),
        _ => {
            for f in [math, text, date, more] {
                match f(name, vals) {
                    Err(Error::Eval(m)) if m.starts_with("unknown function") => continue,
                    other => return other,
                }
            }
            err(format!("unknown function {name}"))
        }
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
        "FIXED" => {
            let d = if vals.len() > 1 { n(vals, 1)? } else { 2.0 };
            let no_commas = vals.len() > 2 && v(vals, 2).truthy()?;
            let x = round_to(n(vals, 0)?, d, f64::round);
            let dec = d.clamp(0.0, 15.0) as usize;
            format(&Value::Number(x), &format!("{}{}{}", if no_commas { "0" } else { "#,##0" }, if dec > 0 { "." } else { "" }, "0".repeat(dec)))
        }
        "UNICHAR" => {
            let code = n(vals, 0)?;
            char::from_u32(code.max(0.0) as u32).map(String::from).ok_or_else(|| Error::Eval("UNICHAR: not a character code".into()))?
        }
        "UNICODE" => return Ok(s(0).chars().next().map(|c| num(c as u32 as f64)).unwrap_or(Value::Blank)),
        "COMBINEVALUES" => {
            let d = s(0);
            vals.iter().skip(1).map(Value::to_text).collect::<Vec<_>>().join(&d)
        }
        "EXACT" => return Ok(Value::Bool(s(0) == s(1))),
        "CONTAINSSTRINGEXACT" => return Ok(Value::Bool(s(0).contains(&s(1)))),
        "REPLACE" => {
            let t: Vec<char> = s(0).chars().collect();
            let start = count(1, 1.0)?.saturating_sub(1).min(t.len());
            let end = (start + count(2, 0.0)?).min(t.len());
            let mut out: String = t.get(..start).unwrap_or_default().iter().collect();
            out.push_str(&s(3));
            out.extend(t.get(end..).unwrap_or_default().iter());
            out
        }
        "SEARCH" | "FIND" => {
            let (needle, hay) = if name == "SEARCH" { (s(0).to_lowercase(), s(1).to_lowercase()) } else { (s(0), s(1)) };
            let start = count(2, 1.0)?.max(1) - 1;
            let hay_chars: Vec<char> = hay.chars().collect();
            let rest: String = hay_chars.get(start.min(hay_chars.len())..).unwrap_or_default().iter().collect();
            let found = if name == "SEARCH" && needle.contains(['*', '?']) { wildcard_find(&rest, &needle) } else { rest.find(&needle).map(|b| rest.get(..b).unwrap_or("").chars().count()) };
            return match found {
                Some(i) => Ok(num((start + i + 1) as f64)),
                None if vals.len() > 3 => Ok(v(vals, 3).clone()),
                None => err(format!("{name}: \"{}\" was not found", s(0))),
            };
        }
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
        "WEEKDAY" => {
            let d = day(0)?;
            match if vals.len() > 1 { n(vals, 1)? as i32 } else { 1 } {
                2 => d.weekday().number_from_monday() as f64,
                3 => d.weekday().num_days_from_monday() as f64,
                _ => (d.weekday().num_days_from_sunday() + 1) as f64,
            }
        }
        "WEEKNUM" => {
            let d = day(0)?;
            let jan1 = chrono::NaiveDate::from_ymd_opt(d.year(), 1, 1).ok_or_else(|| Error::Eval("WEEKNUM: not a date".into()))?;
            let monday = vals.len() > 1 && n(vals, 1)? as i32 == 2;
            let offset = if monday { jan1.weekday().num_days_from_monday() } else { jan1.weekday().num_days_from_sunday() };
            ((d.ordinal0() + offset) / 7 + 1) as f64
        }
        "EDATE" => dates::shift(n(vals, 0)?.floor(), n(vals, 1)? as i32, "MONTH").ok_or_else(|| Error::Eval("EDATE: not a date".into()))?,
        "HOUR" => ((n(vals, 0)?.rem_euclid(1.0) * 24.0) + 1e-9).floor(),
        "MINUTE" => ((n(vals, 0)?.rem_euclid(1.0) * 1440.0) + 1e-9).floor() % 60.0,
        "SECOND" => ((n(vals, 0)?.rem_euclid(1.0) * 86400.0) + 0.5).floor() % 60.0,
        "TIME" => (n(vals, 0)? * 3600.0 + n(vals, 1)? * 60.0 + n(vals, 2)?) / 86400.0,
        "DATEVALUE" => match v(vals, 0) {
            Value::Number(x) => x.floor(),
            other => parse_date_text(&other.to_text()).ok_or_else(|| Error::Eval(format!("DATEVALUE: \"{}\" is not a date", other.to_text())))?.floor(),
        },
        "YEARFRAC" => (n(vals, 1)? - n(vals, 0)?).abs() / 365.25,
        "EOMONTH" => {
            let shifted = dates::shift(n(vals, 0)?, n(vals, 1)? as i32 + 1, "MONTH").and_then(|s| dates::period_start(s, "DATESMTD"));
            shifted.ok_or_else(|| Error::Eval("EOMONTH: not a date".into()))? - 1.0
        }
        _ => return err(format!("unknown function {name}")),
    };
    Ok(num(r))
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

thread_local! {
    static RNG: std::cell::Cell<u64> = std::cell::Cell::new(
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0x9E37_79B9_7F4A_7C15) | 1,
    );
}

fn rand01() -> f64 {
    RNG.with(|r| {
        let mut x = r.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        r.set(x);
        (x >> 11) as f64 / (1u64 << 53) as f64
    })
}

/// Math functions added after the first set.
fn more(name: &str, vals: &[Value]) -> Result<Value> {
    let r = match name {
        "EVEN" | "ODD" => {
            let x = n(vals, 0)?;
            let s = if x < 0.0 { -1.0 } else { 1.0 };
            let mut k = x.abs().ceil();
            let even = (k as i64) % 2 == 0;
            if (name == "EVEN") != even {
                k += 1.0;
            }
            s * k
        }
        "CEILING" | "FLOOR" | "MROUND" | "ISO.CEILING" => {
            let x = n(vals, 0)?;
            let sig = if vals.len() > 1 { n(vals, 1)? } else { 1.0 };
            if sig == 0.0 {
                return Ok(num(0.0));
            }
            let q = x / sig;
            sig * match name {
                "FLOOR" => q.floor(),
                "MROUND" => q.round(),
                _ => q.ceil(),
            }
        }
        "GCD" | "LCM" => {
            let mut acc: u64 = if name == "GCD" { 0 } else { 1 };
            for (i, _) in vals.iter().enumerate() {
                let x = n(vals, i)?.abs().trunc() as u64;
                acc = if name == "GCD" {
                    gcd(acc, x)
                } else if x == 0 || acc == 0 {
                    0
                } else {
                    acc / gcd(acc, x) * x
                };
            }
            acc as f64
        }
        "QUOTIENT" => {
            let d = n(vals, 1)?;
            if d == 0.0 {
                return err("QUOTIENT: division by zero");
            }
            (n(vals, 0)? / d).trunc()
        }
        "RAND" => rand01(),
        "RANDBETWEEN" => {
            let (a, b) = (n(vals, 0)?.ceil(), n(vals, 1)?.floor());
            if b < a {
                return err("RANDBETWEEN: the bottom is above the top");
            }
            a + (rand01() * (b - a + 1.0)).floor()
        }
        "LOG" => {
            let base = if vals.len() > 1 { n(vals, 1)? } else { 10.0 };
            n(vals, 0)?.log(base)
        }
        "FACT" => {
            let k = n(vals, 0)?.trunc();
            if !(0.0..=170.0).contains(&k) {
                return err("FACT: the number is out of range");
            }
            (1..=k as u64).map(|i| i as f64).product()
        }
        "SIN" => n(vals, 0)?.sin(),
        "COS" => n(vals, 0)?.cos(),
        "TAN" => n(vals, 0)?.tan(),
        "ASIN" => n(vals, 0)?.asin(),
        "ACOS" => n(vals, 0)?.acos(),
        "ATAN" => n(vals, 0)?.atan(),
        "DEGREES" => n(vals, 0)?.to_degrees(),
        "RADIANS" => n(vals, 0)?.to_radians(),
        "CURRENCY" => round_to(n(vals, 0)?, 4.0, f64::round),
        "CONVERT" => n(vals, 0)?,
        _ => return err(format!("unknown function {name}")),
    };
    Ok(num(r))
}

/// The 0-based char index of the first match of a `*` / `?` wildcard pattern.
fn wildcard_find(hay: &str, pat: &str) -> Option<usize> {
    fn matches(h: &[char], p: &[char]) -> bool {
        match (p.first(), h.first()) {
            (None, _) => true,
            (Some('*'), _) => matches(h, p.get(1..).unwrap_or_default()) || (!h.is_empty() && matches(h.get(1..).unwrap_or_default(), p)),
            (Some('?'), Some(_)) => matches(h.get(1..).unwrap_or_default(), p.get(1..).unwrap_or_default()),
            (Some(a), Some(b)) if a == b => matches(h.get(1..).unwrap_or_default(), p.get(1..).unwrap_or_default()),
            _ => false,
        }
    }
    let h: Vec<char> = hay.chars().collect();
    let p: Vec<char> = pat.chars().collect();
    (0..=h.len()).find(|i| matches(h.get(*i..).unwrap_or_default(), &p))
}

fn parse_date_text(s: &str) -> Option<f64> {
    let s = s.trim();
    for f in ["%Y-%m-%d", "%Y/%m/%d", "%m/%d/%Y", "%d %b %Y", "%b %d %Y", "%B %d, %Y"] {
        if let Ok(d) = chrono::NaiveDate::parse_from_str(s.get(..10.min(s.len())).filter(|_| f.len() <= 8).unwrap_or(s), f).or_else(|_| chrono::NaiveDate::parse_from_str(s, f)) {
            return dates::to_serial(d);
        }
    }
    None
}

/// FORMAT: named formats ("Currency", "Percent", "Short Date", …), number patterns (`0`,
/// `#,##0.00`, `0.0%`, `$#,##0`, `0.00E+00`, sections `pos;neg;zero`) and date/time patterns
/// (`yyyy-mm-dd`, `mmm yyyy`, `dddd`, `hh:nn:ss AM/PM`).
pub fn format(x: &Value, pattern: &str) -> String {
    let named = match pattern.to_ascii_lowercase().as_str() {
        "" | "general" => return x.to_text(),
        "general number" => "0.##########",
        "currency" => "$#,##0.00",
        "fixed" => "0.00",
        "standard" => "#,##0.00",
        "percent" => "0.00%",
        "scientific" => "0.00E+00",
        "yes/no" => return bool_fmt(x, "Yes", "No"),
        "true/false" => return bool_fmt(x, "True", "False"),
        "on/off" => return bool_fmt(x, "On", "Off"),
        "general date" => "m/d/yyyy h:nn:ss AM/PM",
        "long date" => "dddd, mmmm d, yyyy",
        "medium date" => "dd-mmm-yy",
        "short date" => "m/d/yyyy",
        "long time" => "h:nn:ss AM/PM",
        "medium time" => "hh:nn AM/PM",
        "short time" => "hh:nn",
        _ => pattern,
    };
    let n = match x {
        Value::Number(n) => *n,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Blank => return String::new(),
        Value::Text(t) => match t.trim().parse::<f64>() {
            Ok(n) => n,
            Err(_) => return t.to_string(),
        },
    };
    if is_date_pattern(named) {
        return format_date(n, named);
    }
    // Sections: positive;negative;zero.
    let sections: Vec<&str> = split_sections(named);
    let (pat, val, neg_section) = match sections.as_slice() {
        [_, neg, ..] if n < 0.0 => (*neg, -n, true),
        [_, _, zero, ..] if n == 0.0 => (*zero, n, false),
        [p, ..] => (*p, n, false),
        [] => (named, n, false),
    };
    format_number(val, pat, neg_section)
}

fn bool_fmt(x: &Value, yes: &str, no: &str) -> String {
    match x.truthy() {
        Ok(true) => yes.into(),
        Ok(false) => no.into(),
        Err(_) => x.to_text(),
    }
}

fn split_sections(p: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_q = false;
    for (i, c) in p.char_indices() {
        match c {
            '"' => in_q = !in_q,
            ';' if !in_q => {
                out.push(p.get(start..i).unwrap_or(""));
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(p.get(start..).unwrap_or(""));
    out
}

fn is_date_pattern(p: &str) -> bool {
    let mut in_q = false;
    let mut has_digit_code = false;
    let mut has_date = false;
    for c in p.chars() {
        match c {
            '"' => in_q = !in_q,
            _ if in_q => {}
            'y' | 'Y' | 'd' | 'D' | 'h' | 'H' | 'n' | 'N' | 's' | 'S' | 'm' | 'M' | 'q' | 'Q' => has_date = true,
            '0' | '#' => has_digit_code = true,
            _ => {}
        }
    }
    has_date && !has_digit_code
}

/// A number with a numeric pattern; literal text before and after (`$`, `€`, `"units"`) is kept.
/// `abs_only`: the pattern is a negative section, which shows the magnitude.
fn format_number(val: f64, pat: &str, abs_only: bool) -> String {
    let chars: Vec<char> = pat.chars().collect();
    let is_code = |c: char| matches!(c, '0' | '#' | ',' | '.' | '%' | 'E' | 'e' | '+' | '-');
    let Some(first) = chars.iter().position(|c| matches!(c, '0' | '#')) else {
        return format!("{}{}", literal(pat), Value::Number(val).to_text());
    };
    let mut start = first;
    while start > 0 && chars.get(start - 1).is_some_and(|c| matches!(c, '.' | ',')) {
        start -= 1;
    }
    let mut end = first;
    while chars.get(end).is_some_and(|c| is_code(*c)) {
        // A `-` ends the core unless it follows an exponent marker.
        if *chars.get(end).unwrap_or(&' ') == '-' && !chars.get(end.wrapping_sub(1)).is_some_and(|c| matches!(c, 'E' | 'e')) {
            break;
        }
        end += 1;
    }
    let prefix: String = chars.get(..start).unwrap_or_default().iter().collect();
    let core: String = chars.get(start..end).unwrap_or_default().iter().collect();
    let suffix: String = chars.get(end..).unwrap_or_default().iter().collect();
    let pct = core.contains('%') || literal(&suffix).contains('%');
    let mut x = if pct { val * 100.0 } else { val };
    let core_t = core.trim_end_matches('%');
    // Scaling commas: `#,##0,` divides by 1000 per trailing comma.
    let trailing = core_t.len() - core_t.trim_end_matches(',').len();
    for _ in 0..trailing {
        x /= 1000.0;
    }
    let core_t = core_t.trim_end_matches(',');
    let body = if let Some((mant, exp)) = core_t.split_once(['E', 'e']) {
        let dec = mant.split_once('.').map(|(_, d)| d.chars().filter(|c| matches!(c, '0' | '#')).count()).unwrap_or(0);
        let s = format!("{:.dec$E}", x.abs());
        let (m, e) = s.split_once('E').unwrap_or((&s, "0"));
        let e: i32 = e.parse().unwrap_or(0);
        let digits = exp.chars().filter(|c| *c == '0').count().max(1);
        let sign = if e < 0 {
            "-"
        } else if exp.starts_with('+') {
            "+"
        } else {
            ""
        };
        format!("{}{m}E{sign}{:0digits$}", if x < 0.0 && !abs_only { "-" } else { "" }, e.abs())
    } else {
        let (int_p, frac_p) = core_t.split_once('.').unwrap_or((core_t, ""));
        let max_dec = frac_p.chars().filter(|c| matches!(c, '0' | '#')).count();
        let min_dec = frac_p.chars().filter(|c| *c == '0').count();
        // Round half away from zero first (formatting alone rounds half to even).
        let f = 10f64.powi(max_dec.min(15) as i32);
        let mut s = format!("{:.max_dec$}", (x.abs() * f).round() / f);
        if max_dec > min_dec && s.contains('.') {
            while s.ends_with('0') && s.split_once('.').is_some_and(|(_, f)| f.len() > min_dec) {
                s.pop();
            }
            if s.ends_with('.') {
                s.pop();
            }
        }
        let (int, frac) = s.split_once('.').map(|(a, b)| (a.to_string(), format!(".{b}"))).unwrap_or((s.clone(), String::new()));
        let min_int = int_p.chars().filter(|c| *c == '0').count();
        let mut int = if int == "0" && min_int == 0 { String::new() } else { int };
        while int.len() < min_int {
            int.insert(0, '0');
        }
        if int_p.contains(',') {
            let mut grouped = String::new();
            for (i, ch) in int.chars().enumerate() {
                if i > 0 && (int.len() - i) % 3 == 0 {
                    grouped.push(',');
                }
                grouped.push(ch);
            }
            int = grouped;
        }
        let nonzero = int.chars().any(|c| c.is_ascii_digit() && c != '0') || frac.chars().any(|c| c.is_ascii_digit() && c != '0');
        let neg = x < 0.0 && nonzero && !abs_only;
        format!("{}{int}{frac}", if neg { "-" } else { "" })
    };
    let pct_sign = if core.contains('%') { "%" } else { "" };
    let (sign, body) = match body.strip_prefix('-') {
        Some(b) => ("-", b.to_string()),
        None => ("", body),
    };
    format!("{sign}{}{body}{pct_sign}{}", literal(&prefix), literal(&suffix))
}

/// The literal text of a pattern piece: quotes removed, `\x` escapes resolved.
fn literal(p: &str) -> String {
    let mut out = String::new();
    let mut chars = p.chars();
    let mut in_q = false;
    while let Some(c) = chars.next() {
        match c {
            '"' => in_q = !in_q,
            '\\' if !in_q => {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// A serial date-time with a date pattern.
pub fn format_date(serial: f64, p: &str) -> String {
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    let Some(d) = dates::to_date(serial.floor()) else { return Value::Number(serial).to_text() };
    let secs = ((serial - serial.floor()) * 86400.0).round() as u32;
    let (hh, mi, ss) = (secs / 3600 % 24, secs / 60 % 60, secs % 60);
    let ampm = p.to_ascii_uppercase().contains("AM/PM");
    let chars: Vec<char> = p.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut last_was_hour = false;
    while let Some(&c) = chars.get(i) {
        let run = chars.iter().skip(i).take_while(|x| x.eq_ignore_ascii_case(&c)).count().max(1);
        let lc = c.to_ascii_lowercase();
        match lc {
            '"' => {
                let close = chars.iter().skip(i + 1).position(|x| *x == '"').map(|k| k + i + 1).unwrap_or(chars.len());
                out.extend(chars.get(i + 1..close).unwrap_or_default());
                i = close + 1;
                continue;
            }
            '\\' => {
                if let Some(n) = chars.get(i + 1) {
                    out.push(*n);
                }
                i += 2;
                continue;
            }
            'a' if chars.get(i..i + 5).is_some_and(|w| w.iter().collect::<String>().eq_ignore_ascii_case("AM/PM")) => {
                out.push_str(if hh < 12 { "AM" } else { "PM" });
                i += 5;
                continue;
            }
            'y' => out.push_str(&if run <= 2 { format!("{:02}", d.year() % 100) } else { d.year().to_string() }),
            'q' => out.push_str(&(d.month0() / 3 + 1).to_string()),
            'd' => {
                let wd = DAYS.get(d.weekday().num_days_from_sunday() as usize).copied().unwrap_or("");
                out.push_str(&match run {
                    1 => d.day().to_string(),
                    2 => format!("{:02}", d.day()),
                    3 => wd.get(..3).unwrap_or(wd).to_string(),
                    _ => wd.to_string(),
                })
            }
            'h' => {
                let h = if ampm { (hh + 11) % 12 + 1 } else { hh };
                out.push_str(&if run >= 2 { format!("{h:02}") } else { h.to_string() });
                last_was_hour = true;
                i += run;
                continue;
            }
            'n' => out.push_str(&if run >= 2 { format!("{mi:02}") } else { mi.to_string() }),
            's' => out.push_str(&if run >= 2 { format!("{ss:02}") } else { ss.to_string() }),
            'm' if last_was_hour && run <= 2 => out.push_str(&if run >= 2 { format!("{mi:02}") } else { mi.to_string() }),
            'm' => {
                let name = MONTHS.get(d.month0() as usize).copied().unwrap_or("");
                out.push_str(&match run {
                    1 => d.month().to_string(),
                    2 => format!("{:02}", d.month()),
                    3 => name.get(..3).unwrap_or(name).to_string(),
                    _ => name.to_string(),
                });
            }
            _ => out.extend(chars.get(i..i + run).unwrap_or_default()),
        }
        if !matches!(lc, ':' | ' ') {
            last_was_hour = false;
        }
        i += run;
    }
    out
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
        assert_eq!(format(&Value::Number(1234.5), "$#,##0"), "$1,235");
        assert_eq!(format(&Value::Number(-1234.5), "#,##0.0"), "-1,234.5");
        assert_eq!(format(&Value::Number(0.5), "Percent"), "50.00%");
        assert_eq!(format(&Value::Number(12.0), "0.##"), "12");
        assert_eq!(format(&Value::Number(46023.0), "yyyy-mm-dd"), "2026-01-01");
        assert_eq!(format(&Value::Number(46023.0), "mmm yyyy"), "Jan 2026");
        assert_eq!(format(&Value::Number(46023.75), "dddd hh:nn AM/PM"), "Thursday 06:00 PM");
        assert_eq!(format(&Value::Number(12345.0), "0.00E+00"), "1.23E+04");
        assert_eq!(format(&Value::Number(-5.0), "0;(0)"), "(5)");
    }
}
