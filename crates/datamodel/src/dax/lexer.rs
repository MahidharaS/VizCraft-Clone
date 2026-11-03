//! Tokens of DAX text.

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Number(f64),
    Str(String),
    /// A bare name: function, table, variable or keyword (`VAR`, `RETURN`, `IN`, `NOT`).
    Ident(String),
    /// `'Table name'`.
    Quoted(String),
    /// `[Column or measure]`.
    Bracket(String),
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Op(&'static str),
}

/// Input longer than this is refused (input-derived sizes are capped).
const MAX_LEN: usize = 1 << 20;

pub fn lex(src: &str) -> Result<Vec<Tok>> {
    if src.len() > MAX_LEN {
        return Err(Error::Parse("the expression is too long".into()));
    }
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let at = |i: usize| chars.get(i).copied();
    while let Some(c) = at(i) {
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // Comments: `-- …`, `// …` to the end of the line, `/* … */`.
        if (c == '-' && at(i + 1) == Some('-')) || (c == '/' && at(i + 1) == Some('/')) {
            while at(i).is_some_and(|c| c != '\n') {
                i += 1;
            }
            continue;
        }
        if c == '/' && at(i + 1) == Some('*') {
            i += 2;
            while at(i).is_some() && !(at(i) == Some('*') && at(i + 1) == Some('/')) {
                i += 1;
            }
            i += 2;
            continue;
        }
        match c {
            '0'..='9' | '.' if c != '.' || at(i + 1).is_some_and(|d| d.is_ascii_digit()) => {
                let start = i;
                while at(i).is_some_and(|d| d.is_ascii_digit() || d == '.') {
                    i += 1;
                }
                if at(i).is_some_and(|e| e == 'e' || e == 'E')
                    && at(i + 1).is_some_and(|d| d.is_ascii_digit() || ((d == '+' || d == '-') && at(i + 2).is_some_and(|d| d.is_ascii_digit())))
                {
                    i += 2;
                    while at(i).is_some_and(|d| d.is_ascii_digit()) {
                        i += 1;
                    }
                }
                let text: String = chars.get(start..i).unwrap_or_default().iter().collect();
                let n = text.parse::<f64>().map_err(|_| Error::Parse(format!("not a number: `{text}`")))?;
                out.push(Tok::Number(n));
            }
            '"' => {
                let (s, next) = delimited(&chars, i, '"')?;
                out.push(Tok::Str(s));
                i = next;
            }
            '\'' => {
                let (s, next) = delimited(&chars, i, '\'')?;
                out.push(Tok::Quoted(s));
                i = next;
            }
            '[' => {
                let (s, next) = delimited(&chars, i, ']')?;
                out.push(Tok::Bracket(s));
                i = next;
            }
            '(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            '{' => {
                out.push(Tok::LBrace);
                i += 1;
            }
            '}' => {
                out.push(Tok::RBrace);
                i += 1;
            }
            ',' | ';' => {
                out.push(Tok::Comma);
                i += 1;
            }
            _ if c.is_alphabetic() || c == '_' => {
                let start = i;
                while at(i).is_some_and(|d| d.is_alphanumeric() || d == '_' || d == '.') {
                    i += 1;
                }
                out.push(Tok::Ident(chars.get(start..i).unwrap_or_default().iter().collect()));
            }
            _ => {
                let two: String = [c, at(i + 1).unwrap_or(' ')].iter().collect();
                let op = match two.as_str() {
                    "<=" => Some("<="),
                    ">=" => Some(">="),
                    "<>" => Some("<>"),
                    "==" => Some("=="),
                    "&&" => Some("&&"),
                    "||" => Some("||"),
                    _ => None,
                };
                if let Some(op) = op {
                    out.push(Tok::Op(op));
                    i += 2;
                    continue;
                }
                let op = match c {
                    '+' => "+",
                    '-' => "-",
                    '*' => "*",
                    '/' => "/",
                    '^' => "^",
                    '&' => "&",
                    '=' => "=",
                    '<' => "<",
                    '>' => ">",
                    _ => return Err(Error::Parse(format!("unexpected character `{c}`"))),
                };
                out.push(Tok::Op(op));
                i += 1;
            }
        }
    }
    Ok(out)
}

/// Text between `open` at `i` and its closing delimiter; a doubled closing character escapes it
/// (`"say ""hi"""`, `'O''Brien'`, `[a]]b]`).
fn delimited(chars: &[char], i: usize, close: char) -> Result<(String, usize)> {
    let mut j = i + 1;
    let mut s = String::new();
    loop {
        match chars.get(j) {
            None => return Err(Error::Parse(format!("missing closing `{close}`"))),
            Some(&c) if c == close => {
                if chars.get(j + 1) == Some(&close) {
                    s.push(close);
                    j += 2;
                } else {
                    return Ok((s, j + 1));
                }
            }
            Some(&c) => {
                s.push(c);
                j += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_dax() {
        let t = lex("CALCULATE(SUM(Sales[Amount]), 'Dim Product'[Color] = \"Red\") -- note").unwrap();
        assert_eq!(t.len(), 13);
        assert_eq!(t[4], Tok::Ident("Sales".into()));
        assert_eq!(t[5], Tok::Bracket("Amount".into()));
        assert_eq!(t[8], Tok::Quoted("Dim Product".into()));
        assert!(lex("\"open").is_err());
        assert_eq!(lex("1.5e3 <> 2").unwrap(), vec![Tok::Number(1500.0), Tok::Op("<>"), Tok::Number(2.0)]);
    }
}
