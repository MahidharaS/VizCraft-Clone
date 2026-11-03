//! Tokens of M text.

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Num(f64),
    Text(String),
    /// A name: `Table.AddColumn`, `Source`, `#"Changed Type"`, a keyword.
    Ident(String),
    /// `#date`, `#table` and the other `#` intrinsics.
    Hash(String),
    Punct(&'static str),
}

const MAX_LEN: usize = 4 << 20;

pub fn lex(src: &str) -> Result<Vec<(Tok, usize)>> {
    if src.len() > MAX_LEN {
        return Err(Error::Syntax("the query text is too long".into()));
    }
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let at = |i: usize| chars.get(i).map(|c| c.1);
    let pos = |i: usize| chars.get(i).map(|c| c.0).unwrap_or(src.len());
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(c) = at(i) {
        if c.is_whitespace() || c == '\u{feff}' {
            i += 1;
            continue;
        }
        if c == '/' && at(i + 1) == Some('/') {
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
        let start = pos(i);
        if c.is_ascii_digit() || (c == '.' && at(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            // 0x hex.
            if c == '0' && matches!(at(i + 1), Some('x' | 'X')) {
                let s = i + 2;
                i = s;
                while at(i).is_some_and(|d| d.is_ascii_hexdigit()) {
                    i += 1;
                }
                let text: String = chars.get(s..i).unwrap_or_default().iter().map(|c| c.1).collect();
                let n = i64::from_str_radix(&text, 16).map_err(|_| Error::Syntax(format!("bad number 0x{text}")))?;
                out.push((Tok::Num(n as f64), start));
                continue;
            }
            let s = i;
            while at(i).is_some_and(|d| d.is_ascii_digit()) {
                i += 1;
            }
            // `1..5` is a range, not a decimal point.
            if at(i) == Some('.') && at(i + 1) != Some('.') {
                i += 1;
                while at(i).is_some_and(|d| d.is_ascii_digit()) {
                    i += 1;
                }
            }
            if matches!(at(i), Some('e' | 'E'))
                && (at(i + 1).is_some_and(|d| d.is_ascii_digit())
                    || (matches!(at(i + 1), Some('+' | '-')) && at(i + 2).is_some_and(|d| d.is_ascii_digit())))
            {
                i += 2;
                while at(i).is_some_and(|d| d.is_ascii_digit()) {
                    i += 1;
                }
            }
            let text: String = chars.get(s..i).unwrap_or_default().iter().map(|c| c.1).collect();
            let n = text.parse::<f64>().map_err(|_| Error::Syntax(format!("bad number `{text}`")))?;
            out.push((Tok::Num(n), start));
            continue;
        }
        if c == '"' {
            let (s, next) = text_literal(&chars, i)?;
            out.push((Tok::Text(s), start));
            i = next;
            continue;
        }
        if c == '#' {
            if at(i + 1) == Some('"') {
                let (s, next) = text_literal(&chars, i + 1)?;
                out.push((Tok::Ident(s), start));
                i = next;
                continue;
            }
            let s = i + 1;
            i = s;
            while at(i).is_some_and(|d| d.is_alphanumeric() || d == '_') {
                i += 1;
            }
            let name: String = chars.get(s..i).unwrap_or_default().iter().map(|c| c.1).collect();
            out.push((Tok::Hash(name), start));
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let s = i;
            while at(i).is_some_and(|d| d.is_alphanumeric() || d == '_' || (d == '.' && at(i + 1).is_some_and(|e| e.is_alphanumeric() || e == '_'))) {
                i += 1;
            }
            let name: String = chars.get(s..i).unwrap_or_default().iter().map(|c| c.1).collect();
            out.push((Tok::Ident(name), start));
            continue;
        }
        const PUNCT: [&str; 26] = [
            "=>", "<=", ">=", "<>", "..", "...", "??", "(", ")", "[", "]", "{", "}", ",", ";", "=", "<", ">", "+", "-", "*", "/", "&", "?", "@", "!",
        ];
        let three: String = [c, at(i + 1).unwrap_or(' '), at(i + 2).unwrap_or(' ')].iter().collect();
        if three == "..." {
            out.push((Tok::Punct("..."), start));
            i += 3;
            continue;
        }
        let two: String = [c, at(i + 1).unwrap_or(' ')].iter().collect();
        if let Some(p) = PUNCT.iter().find(|p| p.len() == 2 && **p == two) {
            out.push((Tok::Punct(p), start));
            i += 2;
            continue;
        }
        let one = c.to_string();
        match PUNCT.iter().find(|p| p.len() == 1 && **p == one) {
            Some(p) => {
                out.push((Tok::Punct(p), start));
                i += 1;
            }
            None => return Err(Error::Syntax(format!("unexpected character `{c}`"))),
        }
    }
    Ok(out)
}

/// A `"…"` literal starting at `i`: `""` is a quote, `#(lf)`, `#(cr)`, `#(tab)`, `#(#)` and
/// `#(xxxx)` (hex) are escapes.
fn text_literal(chars: &[(usize, char)], i: usize) -> Result<(String, usize)> {
    let at = |i: usize| chars.get(i).map(|c| c.1);
    let mut j = i + 1;
    let mut s = String::new();
    loop {
        match at(j) {
            None => return Err(Error::Syntax("missing closing quote".into())),
            Some('"') if at(j + 1) == Some('"') => {
                s.push('"');
                j += 2;
            }
            Some('"') => return Ok((s, j + 1)),
            Some('#') if at(j + 1) == Some('(') => {
                let mut k = j + 2;
                let mut name = String::new();
                while let Some(c) = at(k).filter(|c| *c != ')') {
                    name.push(c);
                    k += 1;
                }
                let mut ok = true;
                for part in name.split(',') {
                    match part.trim() {
                        "lf" => s.push('\n'),
                        "cr" => s.push('\r'),
                        "tab" => s.push('\t'),
                        "#" => s.push('#'),
                        hex => match u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
                            Some(c) => s.push(c),
                            None => ok = false,
                        },
                    }
                }
                if ok && at(k) == Some(')') {
                    j = k + 1;
                } else {
                    s.push('#');
                    j += 1;
                }
            }
            Some(c) => {
                s.push(c);
                j += 1;
            }
        }
    }
}
