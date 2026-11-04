//! A small expression language for the Feature Constructor widget:
//! `[Unit Price] * Quantity * (1 - Discount)`, `log(Quantity + 1)`, `if(Channel == "Online", 1, 0)`.
//!
//! Numbers, strings, column references (bare identifiers or `[Any Name]`), `+ - * / % ^`,
//! comparisons, `and / or / not`, and the functions `abs sqrt log ln log10 exp round floor ceil
//! min max pow if year month day len upper lower contains`.

use crate::build::TableData;
use crate::scalar::Scalar;

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    Op(String),
    LParen,
    RParen,
    Comma,
}

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = vec![];
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch.is_ascii_digit() || (ch == '.' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let st = i;
            while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.' || ((c[i] == 'e' || c[i] == 'E') && c.get(i + 1).is_some_and(|d| d.is_ascii_digit() || *d == '-'))) {
                if c[i] == 'e' || c[i] == 'E' {
                    i += 1;
                }
                i += 1;
            }
            let t: String = c[st..i].iter().collect();
            out.push(Tok::Num(t.parse().map_err(|_| format!("Bad number `{t}`."))?));
        } else if ch == '"' || ch == '\'' {
            let q = ch;
            i += 1;
            let st = i;
            while i < c.len() && c[i] != q {
                i += 1;
            }
            if i >= c.len() {
                return Err("Unclosed string.".into());
            }
            out.push(Tok::Str(c[st..i].iter().collect()));
            i += 1;
        } else if ch == '[' {
            i += 1;
            let st = i;
            while i < c.len() && c[i] != ']' {
                i += 1;
            }
            if i >= c.len() {
                return Err("Unclosed [column] reference.".into());
            }
            out.push(Tok::Ident(c[st..i].iter().collect()));
            i += 1;
        } else if ch.is_alphabetic() || ch == '_' {
            let st = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(c[st..i].iter().collect()));
        } else if ch == '(' {
            out.push(Tok::LParen);
            i += 1;
        } else if ch == ')' {
            out.push(Tok::RParen);
            i += 1;
        } else if ch == ',' {
            out.push(Tok::Comma);
            i += 1;
        } else {
            let two: String = c[i..(i + 2).min(c.len())].iter().collect();
            if ["==", "!=", "<=", ">=", "&&", "||", "<>"].contains(&two.as_str()) {
                out.push(Tok::Op(two));
                i += 2;
            } else if "+-*/%^<>=!".contains(ch) {
                out.push(Tok::Op(if ch == '=' { "==".into() } else { ch.to_string() }));
                i += 1;
            } else {
                return Err(format!("Unexpected character `{ch}`."));
            }
        }
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Lit(Scalar),
    Col(String),
    Unary(String, Box<Expr>),
    Bin(String, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

struct Parser {
    t: Vec<Tok>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn op_is(&self, ops: &[&str]) -> Option<String> {
        match self.peek() {
            Some(Tok::Op(o)) if ops.contains(&o.as_str()) => Some(o.clone()),
            Some(Tok::Ident(w)) if ops.contains(&w.to_lowercase().as_str()) => Some(w.to_lowercase()),
            _ => None,
        }
    }
    fn bin(&mut self, level: usize) -> Result<Expr, String> {
        const LEVELS: [&[&str]; 6] = [&["or", "||"], &["and", "&&"], &["==", "!=", "<>"], &["<", "<=", ">", ">="], &["+", "-"], &["*", "/", "%"]];
        if level >= LEVELS.len() {
            return self.unary();
        }
        let mut l = self.bin(level + 1)?;
        while let Some(op) = self.op_is(LEVELS[level]) {
            self.i += 1;
            let r = self.bin(level + 1)?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn unary(&mut self) -> Result<Expr, String> {
        if let Some(op) = self.op_is(&["-", "!", "not", "+"]) {
            self.i += 1;
            let e = self.unary()?;
            return Ok(Expr::Unary(op, Box::new(e)));
        }
        self.power()
    }
    fn power(&mut self) -> Result<Expr, String> {
        let base = self.atom()?;
        if self.op_is(&["^"]).is_some() {
            self.i += 1;
            let e = self.unary()?;
            return Ok(Expr::Bin("^".into(), Box::new(base), Box::new(e)));
        }
        Ok(base)
    }
    fn atom(&mut self) -> Result<Expr, String> {
        let t = self.peek().cloned();
        self.i += 1;
        match t {
            Some(Tok::Num(n)) => Ok(Expr::Lit(Scalar::Num(n))),
            Some(Tok::Str(s)) => Ok(Expr::Lit(Scalar::Text(s))),
            Some(Tok::LParen) => {
                let e = self.bin(0)?;
                if self.peek() != Some(&Tok::RParen) {
                    return Err("Missing `)`.".into());
                }
                self.i += 1;
                Ok(e)
            }
            Some(Tok::Ident(name)) => {
                let lower = name.to_lowercase();
                if lower == "true" || lower == "false" {
                    return Ok(Expr::Lit(Scalar::Bool(lower == "true")));
                }
                if self.peek() == Some(&Tok::LParen) {
                    self.i += 1;
                    let mut args = vec![];
                    if self.peek() != Some(&Tok::RParen) {
                        loop {
                            args.push(self.bin(0)?);
                            match self.peek() {
                                Some(Tok::Comma) => self.i += 1,
                                Some(Tok::RParen) => break,
                                _ => return Err(format!("Expected `,` or `)` in {name}(…).")),
                            }
                        }
                    }
                    self.i += 1;
                    return Ok(Expr::Call(lower, args));
                }
                Ok(Expr::Col(name))
            }
            other => Err(format!("Unexpected {}.", other.map(|t| format!("{t:?}")).unwrap_or_else(|| "end of expression".into()))),
        }
    }
}

pub fn parse(s: &str) -> Result<Expr, String> {
    let t = lex(s)?;
    if t.is_empty() {
        return Err("Empty expression.".into());
    }
    let mut p = Parser { t, i: 0 };
    let e = p.bin(0)?;
    if p.i < p.t.len() {
        return Err(format!("Unexpected {:?} after the expression.", p.t[p.i]));
    }
    Ok(e)
}

fn num(v: &Scalar) -> f64 {
    match v {
        Scalar::Num(n) => *n,
        Scalar::Bool(b) => f64::from(u8::from(*b)),
        Scalar::Text(s) => s.trim().parse().unwrap_or(f64::NAN),
        Scalar::Null => f64::NAN,
    }
}

fn truthy(v: &Scalar) -> bool {
    match v {
        Scalar::Bool(b) => *b,
        Scalar::Num(n) => *n != 0.0 && !n.is_nan(),
        Scalar::Text(s) => !s.is_empty(),
        Scalar::Null => false,
    }
}

fn n(v: f64) -> Scalar {
    if v.is_finite() { Scalar::Num(v) } else { Scalar::Null }
}

/// Evaluates against one row (`get` looks up a column by name).
pub fn eval(e: &Expr, get: &dyn Fn(&str) -> Option<Scalar>) -> Result<Scalar, String> {
    Ok(match e {
        Expr::Lit(v) => v.clone(),
        Expr::Col(c) => get(c).ok_or_else(|| format!("There's no column `{c}`."))?,
        Expr::Unary(op, a) => {
            let v = eval(a, get)?;
            match op.as_str() {
                "-" => n(-num(&v)),
                "+" => v,
                _ => Scalar::Bool(!truthy(&v)),
            }
        }
        Expr::Bin(op, a, b) => {
            let (x, y) = (eval(a, get)?, eval(b, get)?);
            match op.as_str() {
                "+" => match (&x, &y) {
                    (Scalar::Text(s), _) | (_, Scalar::Text(s)) if num(&x).is_nan() || num(&y).is_nan() => {
                        let _ = s;
                        Scalar::Text(format!("{}{}", x.text(), y.text()))
                    }
                    _ => n(num(&x) + num(&y)),
                },
                "-" => n(num(&x) - num(&y)),
                "*" => n(num(&x) * num(&y)),
                "/" => n(num(&x) / num(&y)),
                "%" => n(num(&x) % num(&y)),
                "^" => n(num(&x).powf(num(&y))),
                "and" | "&&" => Scalar::Bool(truthy(&x) && truthy(&y)),
                "or" | "||" => Scalar::Bool(truthy(&x) || truthy(&y)),
                cmp => {
                    let ord = crate::scalar::compare(&x, &y);
                    let eq = if matches!((&x, &y), (Scalar::Num(_), Scalar::Text(_)) | (Scalar::Text(_), Scalar::Num(_))) { x.text() == y.text() } else { ord == std::cmp::Ordering::Equal };
                    Scalar::Bool(match cmp {
                        "==" => eq,
                        "!=" | "<>" => !eq,
                        "<" => ord == std::cmp::Ordering::Less,
                        "<=" => ord != std::cmp::Ordering::Greater,
                        ">" => ord == std::cmp::Ordering::Greater,
                        _ => ord != std::cmp::Ordering::Less,
                    })
                }
            }
        }
        Expr::Call(f, args) => {
            let a: Vec<Scalar> = args.iter().map(|x| eval(x, get)).collect::<Result<_, _>>()?;
            let x = a.first().map(num).unwrap_or(f64::NAN);
            let need = |k: usize| if a.len() < k { Err(format!("{f}() needs {k} argument(s).")) } else { Ok(()) };
            match f.as_str() {
                "abs" => n(x.abs()),
                "sqrt" => n(x.sqrt()),
                "log" | "ln" => n(x.ln()),
                "log10" => n(x.log10()),
                "log2" => n(x.log2()),
                "exp" => n(x.exp()),
                "round" => {
                    let d = a.get(1).map(num).unwrap_or(0.0);
                    let p = 10f64.powi(d as i32);
                    n((x * p).round() / p)
                }
                "floor" => n(x.floor()),
                "ceil" => n(x.ceil()),
                "min" => n(a.iter().map(num).fold(f64::INFINITY, f64::min)),
                "max" => n(a.iter().map(num).fold(f64::NEG_INFINITY, f64::max)),
                "pow" => {
                    need(2)?;
                    n(x.powf(num(&a[1])))
                }
                "if" => {
                    need(3)?;
                    if truthy(&a[0]) { a[1].clone() } else { a[2].clone() }
                }
                "isnull" | "is_missing" => Scalar::Bool(a.first().is_none_or(|v| v.is_null() || matches!(v, Scalar::Num(z) if z.is_nan()))),
                "year" | "month" | "day" | "weekday" => {
                    // Serial dates (days since 1899-12-30).
                    let base = chrono::NaiveDate::from_ymd_opt(1899, 12, 30);
                    let d = base.and_then(|b| if x.is_finite() { b.checked_add_signed(chrono::Duration::days(x.floor() as i64)) } else { None });
                    use chrono::Datelike;
                    match d {
                        Some(d) => Scalar::Num(match f.as_str() {
                            "year" => d.year() as f64,
                            "month" => d.month() as f64,
                            "day" => d.day() as f64,
                            _ => d.weekday().number_from_monday() as f64,
                        }),
                        None => Scalar::Null,
                    }
                }
                "len" => Scalar::Num(a.first().map(|v| v.text().chars().count()).unwrap_or(0) as f64),
                "upper" => Scalar::Text(a.first().map(|v| v.text().to_uppercase()).unwrap_or_default()),
                "lower" => Scalar::Text(a.first().map(|v| v.text().to_lowercase()).unwrap_or_default()),
                "contains" => {
                    need(2)?;
                    Scalar::Bool(a[0].text().to_lowercase().contains(&a[1].text().to_lowercase()))
                }
                "str" | "text" => Scalar::Text(a.first().map(Scalar::text).unwrap_or_default()),
                "num" | "number" => n(x),
                other => return Err(format!("Unknown function `{other}`.")),
            }
        }
    })
}

/// Adds a computed column to a table.
pub fn add_column(t: &TableData, name: &str, expr: &str) -> Result<TableData, String> {
    let e = parse(expr).map_err(|m| format!("{name}: {m}"))?;
    let mut out = t.clone();
    let mut vals = Vec::with_capacity(t.rows.len());
    for r in &t.rows {
        let get = |c: &str| t.column_index(c).map(|i| r.get(i).cloned().unwrap_or(Scalar::Null));
        vals.push(eval(&e, &get).map_err(|m| format!("{name}: {m}"))?);
    }
    let numeric = vals.iter().all(|v| matches!(v, Scalar::Num(_) | Scalar::Null));
    let boolean = vals.iter().all(|v| matches!(v, Scalar::Bool(_) | Scalar::Null)) && vals.iter().any(|v| matches!(v, Scalar::Bool(_)));
    let ty = if boolean { crate::build::ColType::Bool } else if numeric { crate::build::ColType::Number } else { crate::build::ColType::Text };
    let vals: Vec<Scalar> = if boolean { vals.into_iter().map(|v| if let Scalar::Bool(b) = v { Scalar::Text(if b { "yes".into() } else { "no".into() }) } else { v }).collect() } else { vals };
    let ty = if boolean { crate::build::ColType::Text } else { ty };
    crate::flow::set_column(&mut out, name, vals, ty);
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn expressions() {
        let get = |c: &str| match c {
            "Quantity" => Some(Scalar::Num(3.0)),
            "Unit Price" => Some(Scalar::Num(10.0)),
            "Discount" => Some(Scalar::Num(0.1)),
            "Channel" => Some(Scalar::Text("Online".into())),
            _ => None,
        };
        let ev = |s: &str| eval(&parse(s).unwrap(), &get).unwrap();
        assert_eq!(ev("[Unit Price] * Quantity * (1 - Discount)"), Scalar::Num(27.0));
        assert_eq!(ev("2 ^ 3 ^ 2"), Scalar::Num(512.0));
        assert_eq!(ev("-2 ^ 2"), Scalar::Num(-4.0));
        assert_eq!(ev("if(Channel == \"Online\", 1, 0)"), Scalar::Num(1.0));
        assert_eq!(ev("Quantity > 2 and not (Discount > 0.5)"), Scalar::Bool(true));
        assert_eq!(ev("round(log(100) / log(10), 2)"), Scalar::Num(2.0));
        assert_eq!(ev("year(45000)"), Scalar::Num(2023.0));
        assert!(parse("1 +").is_err());
        assert!(eval(&parse("Nope + 1").unwrap(), &get).is_err());
    }
}
