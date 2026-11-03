//! `Number.*` functions and numeric conversions.

use super::convert::{number_from, parse_number, to_type};
use super::{arg, err, num_arg, opt_num};
use crate::Result;
use crate::m::eval::Evaluator;
use crate::value::{MType, Value, number_text};

/// Rounds to `digits` decimals. `mode`: 0 ceiling, 1 floor, 2 truncate, 3 away from zero, and
/// for ties only: 10 up, 11 down, 12 away from zero, 13 toward zero, 14 to even.
fn round_to(x: f64, digits: f64, mode: u8) -> f64 {
    let f = 10f64.powi(digits.clamp(-15.0, 15.0) as i32);
    let y = x * f;
    let r = match mode {
        0 => y.ceil(),
        1 => y.floor(),
        2 => y.trunc(),
        3 => {
            if y >= 0.0 {
                y.ceil()
            } else {
                y.floor()
            }
        }
        10 => (y + 0.5).floor(),
        11 => (y - 0.5).ceil(),
        12 => y.round(),
        13 => {
            if (y.fract().abs() - 0.5).abs() < 1e-12 {
                y.trunc()
            } else {
                y.round()
            }
        }
        _ => y.round_ties_even(),
    };
    r / f
}

/// `Number.ToText(n, "N2")` style formats: N (thousands), F (fixed), P (percent), D (digits), E.
pub fn format_number(n: f64, fmt: &str) -> String {
    let (kind, digits) = fmt.split_at(fmt.chars().next().map(char::len_utf8).unwrap_or(0));
    let d: usize = digits.parse().unwrap_or(2);
    let group = |s: String| -> String {
        let (sign, body) = s.strip_prefix('-').map(|b| ("-", b.to_string())).unwrap_or(("", s.clone()));
        let (int, frac) = body.split_once('.').map(|(i, f)| (i.to_string(), format!(".{f}"))).unwrap_or((body.clone(), String::new()));
        let mut out = String::new();
        for (i, c) in int.chars().enumerate() {
            if i > 0 && (int.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        format!("{sign}{out}{frac}")
    };
    match kind {
        "N" | "n" => group(format!("{n:.d$}")),
        "F" | "f" => format!("{n:.d$}"),
        "P" | "p" => format!("{} %", group(format!("{:.d$}", n * 100.0))),
        "D" | "d" => format!("{:0d$}", n.round() as i64),
        "E" | "e" => format!("{n:.d$e}"),
        "C" | "c" => format!("${}", group(format!("{n:.d$}"))),
        _ => number_text(n),
    }
}

pub fn call(_ev: &Evaluator, name: &str, a: &[Value]) -> Option<Result<Value>> {
    let unary = |f: fn(f64) -> f64| -> Result<Value> {
        match arg(a, 0) {
            Value::Null => Ok(Value::Null),
            _ => Ok(Value::Number(f(num_arg(a, 0, name)?))),
        }
    };
    let r = match name {
        "Number.From" | "Double.From" | "Decimal.From" => Ok(match arg(a, 0) {
            Value::Null => Value::Null,
            v => number_from(v)
                .map(Value::Number)
                .unwrap_or_else(|| Value::error(format!("DataFormat.Error: We couldn't convert to Number. Details: {}", v.display()))),
        }),
        "Int64.From" => Ok(to_type(arg(a, 0), MType::Int64)),
        "Currency.From" => Ok(to_type(arg(a, 0), MType::Currency)),
        "Percentage.From" => Ok(to_type(arg(a, 0), MType::Percentage)),
        "Number.FromText" => match arg(a, 0) {
            Value::Null => Ok(Value::Null),
            Value::Text(t) => parse_number(t)
                .map(Value::Number)
                .ok_or_else(|| crate::Error::Eval(format!("DataFormat.Error: We couldn't convert to Number. Details: {t}"))),
            other => err(format!("expected text, got {}", other.kind())),
        },
        "Number.ToText" => match arg(a, 0) {
            Value::Null => Ok(Value::Null),
            _ => num_arg(a, 0, name).map(|n| {
                Value::text(match arg(a, 1).as_text() {
                    Some(f) => format_number(n, f),
                    None => number_text(n),
                })
            }),
        },
        "Number.Abs" => unary(f64::abs),
        "Number.Sqrt" => unary(f64::sqrt),
        "Number.Exp" => unary(f64::exp),
        "Number.Ln" => unary(f64::ln),
        "Number.Log10" => unary(f64::log10),
        "Number.Sign" => unary(|x| if x == 0.0 { 0.0 } else { x.signum() }),
        "Number.Log" => match arg(a, 0) {
            Value::Null => Ok(Value::Null),
            _ => num_arg(a, 0, name).map(|x| {
                Value::Number(match opt_num(a, 1) {
                    Some(b) => x.log(b),
                    None => x.ln(),
                })
            }),
        },
        "Number.Power" => match arg(a, 0) {
            Value::Null => Ok(Value::Null),
            _ => num_arg(a, 0, name).and_then(|x| Ok(Value::Number(x.powf(num_arg(a, 1, name)?)))),
        },
        "Number.Mod" | "Number.IntegerDivide" => match arg(a, 0) {
            Value::Null => Ok(Value::Null),
            _ => (|| {
                let x = num_arg(a, 0, name)?;
                let y = num_arg(a, 1, name)?;
                if y == 0.0 {
                    return Ok(Value::Number(f64::NAN));
                }
                Ok(Value::Number(if name == "Number.Mod" { x - y * (x / y).trunc() } else { (x / y).trunc() }))
            })(),
        },
        "Number.Round" | "Number.RoundUp" | "Number.RoundDown" | "Number.RoundTowardZero" | "Number.RoundAwayFromZero" => match arg(a, 0) {
            Value::Null => Ok(Value::Null),
            _ => num_arg(a, 0, name).map(|x| {
                let digits = opt_num(a, 1).unwrap_or(0.0);
                let mode = match name {
                    "Number.RoundUp" => 0,
                    "Number.RoundDown" => 1,
                    "Number.RoundTowardZero" => 2,
                    "Number.RoundAwayFromZero" => 3,
                    _ => match opt_num(a, 2).map(|m| m as u8) {
                        Some(0) => 10,
                        Some(1) => 11,
                        Some(2) => 12,
                        Some(3) => 13,
                        _ => 14,
                    },
                };
                Value::Number(round_to(x, digits, mode))
            }),
        },
        "Number.IsEven" | "Number.IsOdd" => num_arg(a, 0, name).map(|x| Value::Logical((x.rem_euclid(2.0) == 0.0) == (name == "Number.IsEven"))),
        "Number.IsNaN" => num_arg(a, 0, name).map(|x| Value::Logical(x.is_nan())),
        _ => return None,
    };
    Some(r)
}
