//! Data sheets: an Excel-like grid with a formula engine.
//!
//! Cells hold raw input (`cells[row][col]`, row 0 = headers). Input starting with `=` is a
//! formula: A1 references (`B2`, `$B$2`, `A2:C10`, `A:A`, `Sheet2!A1`, `'My sheet'!A1:B3`),
//! operators (`+ - * / ^ & %`, comparisons), and ~70 Excel functions. Evaluation is lazy with
//! memoisation and cycle detection; errors are Excel's (`#DIV/0!`, `#REF!`, `#NAME?`,
//! `#VALUE!`, `#N/A`, `#NUM!`, `#CIRC!`).

use std::collections::HashMap;
use std::rc::Rc;

use crate::build::{ColType, TableData};
use crate::doc::DataSheet;
use crate::scalar::Scalar;

// =============================================================================================
// Public API
// =============================================================================================

/// An evaluated cell.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum CellValue {
    #[default]
    Empty,
    Num(f64),
    Text(String),
    Bool(bool),
    Error(String),
}

impl CellValue {
    pub fn is_num(&self) -> bool {
        matches!(self, CellValue::Num(_))
    }
    pub fn is_error(&self) -> bool {
        matches!(self, CellValue::Error(_))
    }
    /// Display text with an optional number format (`#,0.00`, `$#,0`, `0%`, `yyyy-mm-dd`).
    pub fn display(&self, fmt: &str) -> String {
        match self {
            CellValue::Empty => String::new(),
            CellValue::Num(n) => {
                if fmt.is_empty() {
                    crate::numfmt::general(*n)
                } else if crate::numfmt::is_date_format(fmt) {
                    crate::numfmt::format_date(*n, fmt)
                } else {
                    crate::numfmt::format(*n, fmt)
                }
            }
            CellValue::Text(t) => t.clone(),
            CellValue::Bool(b) => if *b { "TRUE".into() } else { "FALSE".into() },
            CellValue::Error(e) => e.clone(),
        }
    }
    pub fn to_scalar(&self) -> Scalar {
        match self {
            CellValue::Empty | CellValue::Error(_) => Scalar::Null,
            CellValue::Num(n) => Scalar::Num(*n),
            CellValue::Text(t) => Scalar::Text(t.clone()),
            CellValue::Bool(b) => Scalar::Bool(*b),
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            CellValue::Num(n) => Some(*n),
            CellValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => None,
        }
    }
}

/// Evaluates a sheet to table data (row 0 = headers). References to other sheets give `#REF!`;
/// use [`evaluate_with`] to resolve them.
pub fn evaluate(s: &DataSheet) -> TableData {
    evaluate_with(s, std::slice::from_ref(s))
}

/// Evaluates a sheet to table data, resolving `Sheet!A1` references against `book`.
pub fn evaluate_with(s: &DataSheet, book: &[DataSheet]) -> TableData {
    let grid = evaluate_grid_with(s, book);
    table_from_grid(s, &grid)
}

/// Turns an evaluated grid into table data, honouring the sheet's column types.
pub fn table_from_grid(s: &DataSheet, grid: &[Vec<CellValue>]) -> TableData {
    let header_row = grid.first().cloned().unwrap_or_default();
    let mut width = header_row.iter().rposition(|v| !matches!(v, CellValue::Empty)).map(|i| i + 1).unwrap_or(0);
    if width == 0 {
        width = grid.iter().map(|r| r.len()).max().unwrap_or(0);
    }
    let mut headers: Vec<String> = Vec::with_capacity(width);
    for c in 0..width {
        let mut h = header_row.get(c).map(|v| v.display("")).unwrap_or_default().trim().to_string();
        if h.is_empty() {
            h = format!("Column{}", c + 1);
        }
        let base = h.clone();
        let mut k = 2;
        while headers.iter().any(|x| x.eq_ignore_ascii_case(&h)) {
            h = format!("{base}{k}");
            k += 1;
        }
        headers.push(h);
    }
    let types: Vec<ColType> = (0..width).map(|c| s.col_types.get(c).map(|t| ColType::from_label(t)).unwrap_or(ColType::Any)).collect();
    let rows: Vec<Vec<Scalar>> = grid
        .iter()
        .skip(1)
        .filter(|r| r.iter().take(width).any(|c| !matches!(c, CellValue::Empty)))
        .map(|r| {
            (0..width)
                .map(|c| {
                    let v = r.get(c).map(CellValue::to_scalar).unwrap_or(Scalar::Null);
                    match types.get(c).copied().unwrap_or_default() {
                        ColType::Any => v,
                        ty => crate::build::convert(&v, ty),
                    }
                })
                .collect()
        })
        .collect();
    let mut t = TableData { name: s.name.clone(), columns: headers, types, rows };
    t.infer_types();
    t
}

/// Evaluates every cell of a sheet (the grid the UI shows).
pub fn evaluate_grid(s: &DataSheet) -> Vec<Vec<CellValue>> {
    evaluate_grid_with(s, std::slice::from_ref(s))
}

/// Evaluates every cell of `s`; other sheets of `book` are reachable by name.
pub fn evaluate_grid_with(s: &DataSheet, book: &[DataSheet]) -> Vec<Vec<CellValue>> {
    let mut sheets: Vec<&DataSheet> = vec![s];
    sheets.extend(book.iter().filter(|b| !std::ptr::eq(*b as *const DataSheet, s as *const DataSheet) && !b.name.eq_ignore_ascii_case(&s.name)));
    let mut ev = Book::new(sheets);
    let rows = s.cells.len();
    let mut out = Vec::with_capacity(rows);
    for r in 0..rows {
        let w = s.cells.get(r).map(|x| x.len()).unwrap_or(0);
        let mut row = Vec::with_capacity(w);
        for c in 0..w {
            row.push(ev.cell(0, r, c).to_cell());
        }
        out.push(row);
    }
    out
}

/// Evaluates one formula (or raw input) in the context of a sheet, as if typed in `(row, col)`.
pub fn evaluate_formula(s: &DataSheet, book: &[DataSheet], input: &str) -> CellValue {
    let mut sheets: Vec<&DataSheet> = vec![s];
    sheets.extend(book.iter().filter(|b| !b.name.eq_ignore_ascii_case(&s.name)));
    let mut ev = Book::new(sheets);
    match input.strip_prefix('=') {
        Some(f) => match parse_formula(f) {
            Ok(e) => match ev.eval(0, &e).scalar() {
                V::Empty => CellValue::Num(0.0),
                v => v.to_cell(),
            },
            Err(e) => CellValue::Error(e),
        },
        None => input_value(input).to_cell(),
    }
}

/// A raw cell input as a value: numbers, booleans, dates, text.
pub fn parse_input(s: &str) -> Scalar {
    input_value(s).to_cell().to_scalar()
}

/// Column letters for a 0-based column index (0 → A, 26 → AA).
pub fn col_name(mut c: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push((b'A' + (c % 26) as u8) as char);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    s.iter().rev().collect()
}

/// A 0-based column index for letters (A → 0).
pub fn col_index(letters: &str) -> Option<usize> {
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let mut n: usize = 0;
    for ch in letters.chars() {
        if !ch.is_ascii_alphabetic() {
            return None;
        }
        n = n * 26 + (ch.to_ascii_uppercase() as u8 - b'A' + 1) as usize;
    }
    Some(n - 1)
}

/// The A1 address of a 0-based (row, col).
pub fn address(row: usize, col: usize) -> String {
    format!("{}{}", col_name(col), row + 1)
}

/// Parses an A1 address (`B3`, `$B$3`) to 0-based (row, col).
pub fn parse_address(s: &str) -> Option<(usize, usize)> {
    let r = parse_ref_word(s.trim())?;
    Some((r.row, r.col))
}

/// Shifts the relative references of a formula by (rows, cols), like Excel does when a formula
/// is copied or filled. References that fall off the sheet become `#REF!`. Non-formulas are
/// returned unchanged.
pub fn shift_formula(input: &str, dr: i64, dc: i64) -> String {
    let Some(f) = input.strip_prefix('=') else { return input.to_string() };
    let Ok(toks) = lex(f) else { return input.to_string() };
    let mut out = String::from("=");
    let mut last = 0;
    for (i, t) in toks.iter().enumerate() {
        if let Tok::Word(w) = &t.tok {
            let next_is_paren = matches!(toks.get(i + 1).map(|x| &x.tok), Some(Tok::LParen));
            let adj_colon = matches!(toks.get(i + 1).map(|x| &x.tok), Some(Tok::Colon)) || (i > 0 && matches!(toks.get(i - 1).map(|x| &x.tok), Some(Tok::Colon)));
            let new = if next_is_paren {
                None
            } else if let Some(r) = parse_ref_word(w) {
                let row = if r.row_abs { Some(r.row as i64) } else { Some(r.row as i64 + dr) };
                let col = if r.col_abs { Some(r.col as i64) } else { Some(r.col as i64 + dc) };
                match (row, col) {
                    (Some(rr), Some(cc)) if rr >= 0 && cc >= 0 => Some(format!(
                        "{}{}{}{}",
                        if r.col_abs { "$" } else { "" },
                        col_name(cc as usize),
                        if r.row_abs { "$" } else { "" },
                        rr + 1
                    )),
                    _ => Some("#REF!".to_string()),
                }
            } else if adj_colon && let Some((abs, c)) = parse_col_word(w) {
                let cc = if abs { c as i64 } else { c as i64 + dc };
                if cc >= 0 { Some(format!("{}{}", if abs { "$" } else { "" }, col_name(cc as usize))) } else { Some("#REF!".into()) }
            } else {
                None
            };
            if let Some(n) = new {
                out.push_str(f.get(last..t.start).unwrap_or(""));
                out.push_str(&n);
                last = t.end;
            }
        }
    }
    out.push_str(f.get(last..).unwrap_or(""));
    out
}

/// The function names the engine knows (for autocomplete and help).
pub const FUNCTIONS: &[&str] = &[
    "ABS", "AND", "AVERAGE", "AVERAGEIF", "CEILING", "CHAR", "CHOOSE", "CONCAT", "CONCATENATE", "COUNT", "COUNTA", "COUNTBLANK", "COUNTIF", "COUNTIFS", "DATE", "DAY", "DAYS", "EDATE", "EOMONTH", "EXACT", "EXP", "FALSE", "FIND", "FLOOR", "HOUR",
    "IF", "IFERROR", "IFNA", "IFS", "INDEX", "INT", "ISBLANK", "ISERROR", "ISNUMBER", "ISTEXT", "LEFT", "LEN", "LN", "LOG", "LOG10", "LOWER", "MATCH", "MAX", "MEDIAN", "MID", "MIN", "MINUTE", "MOD", "MONTH", "NOT", "NOW", "OR", "PI", "POWER",
    "PRODUCT", "PROPER", "RAND", "REPT", "RIGHT", "ROUND", "ROUNDDOWN", "ROUNDUP", "SEARCH", "SECOND", "SIGN", "SQRT", "STDEV", "SUBSTITUTE", "SUM", "SUMIF", "SUMIFS", "SUMPRODUCT", "SWITCH", "TEXT", "TEXTJOIN", "TODAY", "TRIM", "TRUE",
    "UPPER", "VALUE", "VAR", "VLOOKUP", "HLOOKUP", "WEEKDAY", "XLOOKUP", "YEAR",
];

// =============================================================================================
// Values
// =============================================================================================

#[derive(Clone, Debug, PartialEq)]
enum V {
    Empty,
    Num(f64),
    Text(Rc<str>),
    Bool(bool),
    Err(&'static str),
    Arr(Rc<Arr>),
}

#[derive(Clone, Debug, PartialEq)]
struct Arr {
    rows: usize,
    cols: usize,
    data: Vec<V>,
}

impl Arr {
    fn get(&self, r: usize, c: usize) -> V {
        self.data.get(r * self.cols + c).cloned().unwrap_or(V::Empty)
    }
}

const DIV0: &str = "#DIV/0!";
const REF: &str = "#REF!";
const NAME: &str = "#NAME?";
const VALUE: &str = "#VALUE!";
const NA: &str = "#N/A";
const NUM: &str = "#NUM!";
const CIRC: &str = "#CIRC!";

impl V {
    fn text(s: impl Into<String>) -> V {
        V::Text(Rc::from(s.into().as_str()))
    }
    fn scalar(self) -> V {
        match self {
            V::Arr(a) => a.data.first().cloned().unwrap_or(V::Empty),
            v => v,
        }
    }
    fn to_cell(&self) -> CellValue {
        match self {
            V::Empty => CellValue::Empty,
            V::Num(n) => {
                if n.is_finite() {
                    CellValue::Num(*n)
                } else {
                    CellValue::Error(NUM.into())
                }
            }
            V::Text(t) => CellValue::Text(t.to_string()),
            V::Bool(b) => CellValue::Bool(*b),
            V::Err(e) => CellValue::Error((*e).into()),
            V::Arr(a) => a.data.first().map(|v| v.to_cell()).unwrap_or_default(),
        }
    }
    fn num(&self) -> Result<f64, &'static str> {
        match self {
            V::Num(n) => Ok(*n),
            V::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            V::Empty => Ok(0.0),
            V::Text(t) => text_to_num(t).ok_or(VALUE),
            V::Err(e) => Err(e),
            V::Arr(a) => a.data.first().cloned().unwrap_or(V::Empty).num(),
        }
    }
    fn str(&self) -> Result<String, &'static str> {
        match self {
            V::Num(n) => Ok(crate::numfmt::general(*n)),
            V::Bool(b) => Ok(if *b { "TRUE".into() } else { "FALSE".into() }),
            V::Empty => Ok(String::new()),
            V::Text(t) => Ok(t.to_string()),
            V::Err(e) => Err(e),
            V::Arr(a) => a.data.first().cloned().unwrap_or(V::Empty).str(),
        }
    }
    fn bool(&self) -> Result<bool, &'static str> {
        match self {
            V::Bool(b) => Ok(*b),
            V::Num(n) => Ok(*n != 0.0),
            V::Empty => Ok(false),
            V::Text(t) => match t.trim().to_ascii_uppercase().as_str() {
                "TRUE" => Ok(true),
                "FALSE" => Ok(false),
                _ => Err(VALUE),
            },
            V::Err(e) => Err(e),
            V::Arr(a) => a.data.first().cloned().unwrap_or(V::Empty).bool(),
        }
    }
    fn is_err(&self) -> bool {
        matches!(self, V::Err(_))
    }
    /// The values of an argument, flattened (ranges and arrays row by row).
    fn flat(&self) -> Vec<V> {
        match self {
            V::Arr(a) => a.data.clone(),
            v => vec![v.clone()],
        }
    }
    fn dims(&self) -> (usize, usize) {
        match self {
            V::Arr(a) => (a.rows, a.cols),
            _ => (1, 1),
        }
    }
}

fn text_to_num(t: &str) -> Option<f64> {
    let s = t.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(n) = parse_number(s) {
        return Some(n);
    }
    crate::build::parse_date(s)
}

/// Numbers as typed: `1234`, `1,234.5`, `$12`, `-€3.5`, `12%`, `(42)`.
fn parse_number(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (neg_paren, s) = match s.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
        Some(inner) => (true, inner.trim()),
        None => (false, s),
    };
    let (pct, s) = match s.strip_suffix('%') {
        Some(x) => (true, x.trim()),
        None => (false, s),
    };
    let (neg, s) = match s.strip_prefix('-') {
        Some(x) => (true, x.trim()),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let s = s.trim_start_matches(['$', '€', '£', '¥']);
    let (neg2, s) = match s.strip_prefix('-') {
        Some(x) => (true, x),
        None => (false, s),
    };
    if s.is_empty() || !s.chars().next().is_some_and(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    // Thousands separators only between digit groups.
    let cleaned: String = if s.contains(',') {
        let int = s.split('.').next().unwrap_or("");
        let groups: Vec<&str> = int.split(',').collect();
        if groups.iter().skip(1).any(|g| g.len() != 3) || groups.first().is_some_and(|g| g.is_empty() || g.len() > 3) {
            return None;
        }
        s.replace(',', "")
    } else {
        s.to_string()
    };
    if cleaned.eq_ignore_ascii_case("inf") || cleaned.eq_ignore_ascii_case("nan") || cleaned.to_ascii_lowercase().contains("inf") {
        return None;
    }
    let mut n: f64 = cleaned.parse().ok()?;
    if pct {
        n /= 100.0;
    }
    if neg ^ neg2 ^ neg_paren {
        n = -n;
    }
    Some(n)
}

fn input_value(s: &str) -> V {
    if let Some(t) = s.strip_prefix('\'') {
        return V::text(t);
    }
    let t = s.trim();
    if t.is_empty() {
        return V::Empty;
    }
    if let Some(n) = parse_number(t) {
        return V::Num(n);
    }
    match t.to_ascii_uppercase().as_str() {
        "TRUE" => return V::Bool(true),
        "FALSE" => return V::Bool(false),
        _ => {}
    }
    for e in [DIV0, REF, NAME, VALUE, NA, NUM] {
        if t.eq_ignore_ascii_case(e) {
            return V::Err(e);
        }
    }
    V::text(s)
}

// =============================================================================================
// Lexer
// =============================================================================================

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Word(String),
    Sheet(String),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
    Colon,
    LBrace,
    RBrace,
    Semi,
    ErrLit(&'static str),
}

#[derive(Clone, Debug)]
struct Spanned {
    tok: Tok,
    start: usize,
    end: usize,
}

fn lex(src: &str) -> Result<Vec<Spanned>, String> {
    let b: Vec<(usize, char)> = src.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let at = |i: usize| b.get(i).map(|x| x.1);
    let pos = |i: usize| b.get(i).map(|x| x.0).unwrap_or(src.len());
    while i < b.len() {
        let c = at(i).unwrap_or(' ');
        let start = pos(i);
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let single = |t: Tok| Spanned { tok: t, start, end: start + c.len_utf8() };
        match c {
            '(' => {
                out.push(single(Tok::LParen));
                i += 1;
            }
            ')' => {
                out.push(single(Tok::RParen));
                i += 1;
            }
            ',' => {
                out.push(single(Tok::Comma));
                i += 1;
            }
            ';' => {
                out.push(single(Tok::Semi));
                i += 1;
            }
            ':' => {
                out.push(single(Tok::Colon));
                i += 1;
            }
            '{' => {
                out.push(single(Tok::LBrace));
                i += 1;
            }
            '}' => {
                out.push(single(Tok::RBrace));
                i += 1;
            }
            '+' | '-' | '*' | '/' | '^' | '&' | '%' | '=' => {
                let op = match c {
                    '+' => "+",
                    '-' => "-",
                    '*' => "*",
                    '/' => "/",
                    '^' => "^",
                    '&' => "&",
                    '%' => "%",
                    _ => "=",
                };
                out.push(single(Tok::Op(op)));
                i += 1;
            }
            '<' | '>' => {
                let n = at(i + 1);
                let (op, len) = match (c, n) {
                    ('<', Some('=')) => ("<=", 2),
                    ('<', Some('>')) => ("<>", 2),
                    ('>', Some('=')) => (">=", 2),
                    ('<', _) => ("<", 1),
                    _ => (">", 1),
                };
                out.push(Spanned { tok: Tok::Op(op), start, end: pos(i + len) });
                i += len;
            }
            '"' => {
                let mut s = String::new();
                i += 1;
                loop {
                    match at(i) {
                        None => return Err("#VALUE! (unclosed text)".into()),
                        Some('"') if at(i + 1) == Some('"') => {
                            s.push('"');
                            i += 2;
                        }
                        Some('"') => {
                            i += 1;
                            break;
                        }
                        Some(ch) => {
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                out.push(Spanned { tok: Tok::Str(s), start, end: pos(i) });
            }
            '\'' => {
                let mut s = String::new();
                i += 1;
                loop {
                    match at(i) {
                        None => return Err("#REF! (unclosed sheet name)".into()),
                        Some('\'') if at(i + 1) == Some('\'') => {
                            s.push('\'');
                            i += 2;
                        }
                        Some('\'') => {
                            i += 1;
                            break;
                        }
                        Some(ch) => {
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                if at(i) != Some('!') {
                    return Err("#REF! (expected ! after sheet name)".into());
                }
                i += 1;
                out.push(Spanned { tok: Tok::Sheet(s), start, end: pos(i) });
            }
            '#' => {
                let rest = src.get(start..).unwrap_or("");
                let mut found = None;
                for e in [DIV0, REF, NAME, VALUE, NA, NUM] {
                    if rest.len() >= e.len() && rest.get(..e.len()).is_some_and(|x| x.eq_ignore_ascii_case(e)) {
                        found = Some(e);
                        break;
                    }
                }
                let Some(e) = found else { return Err(NAME.into()) };
                out.push(Spanned { tok: Tok::ErrLit(e), start, end: start + e.len() });
                while pos(i) < start + e.len() {
                    i += 1;
                }
            }
            c if c.is_ascii_digit() || (c == '.' && at(i + 1).is_some_and(|d| d.is_ascii_digit())) => {
                let s0 = i;
                while at(i).is_some_and(|d| d.is_ascii_digit() || d == '.') {
                    i += 1;
                }
                if at(i).is_some_and(|d| d == 'e' || d == 'E') && at(i + 1).is_some_and(|d| d.is_ascii_digit() || d == '+' || d == '-') {
                    i += 2;
                    while at(i).is_some_and(|d| d.is_ascii_digit()) {
                        i += 1;
                    }
                }
                let text = src.get(pos(s0)..pos(i)).unwrap_or("");
                let n: f64 = text.parse().map_err(|_| format!("{VALUE} (bad number {text})"))?;
                out.push(Spanned { tok: Tok::Num(n), start, end: pos(i) });
            }
            c if c.is_alphabetic() || c == '_' || c == '$' => {
                let s0 = i;
                while at(i).is_some_and(|d| d.is_alphanumeric() || d == '_' || d == '.' || d == '$') {
                    i += 1;
                }
                let w = src.get(pos(s0)..pos(i)).unwrap_or("").to_string();
                if at(i) == Some('!') {
                    i += 1;
                    out.push(Spanned { tok: Tok::Sheet(w), start, end: pos(i) });
                } else {
                    out.push(Spanned { tok: Tok::Word(w), start, end: pos(i) });
                }
            }
            other => return Err(format!("{NAME} (unexpected `{other}`)")),
        }
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RefWord {
    row: usize,
    col: usize,
    row_abs: bool,
    col_abs: bool,
}

/// `B3`, `$B3`, `B$3`, `$B$3` → 0-based.
fn parse_ref_word(w: &str) -> Option<RefWord> {
    let mut s = w;
    let col_abs = s.starts_with('$');
    s = s.trim_start_matches('$');
    let letters_end = s.find(|c: char| !c.is_ascii_alphabetic())?;
    let (letters, rest) = s.split_at(letters_end);
    let row_abs = rest.starts_with('$');
    let digits = rest.trim_start_matches('$');
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) || digits.len() > 7 {
        return None;
    }
    let col = col_index(letters)?;
    let row: usize = digits.parse().ok()?;
    if row == 0 {
        return None;
    }
    Some(RefWord { row: row - 1, col, row_abs, col_abs })
}

/// `B` / `$B` (whole-column references like `A:A`).
fn parse_col_word(w: &str) -> Option<(bool, usize)> {
    let abs = w.starts_with('$');
    let s = w.trim_start_matches('$');
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some((abs, col_index(s)?))
}

// =============================================================================================
// Parser
// =============================================================================================

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Num(f64),
    Str(String),
    Bool(bool),
    Err(&'static str),
    Ref { sheet: Option<String>, row: usize, col: usize },
    Range { sheet: Option<String>, r1: usize, c1: usize, r2: usize, c2: usize },
    Neg(Box<Expr>),
    Percent(Box<Expr>),
    Bin(&'static str, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    Array(Vec<Vec<Expr>>),
    /// An unknown name (gives #NAME?).
    Name(String),
}

fn parse_formula(src: &str) -> Result<Expr, String> {
    let toks = lex(src)?;
    let mut p = Parser { toks, i: 0 };
    let e = p.expr(0)?;
    if p.i < p.toks.len() {
        return Err(format!("{VALUE} (unexpected token)"));
    }
    Ok(e)
}

struct Parser {
    toks: Vec<Spanned>,
    i: usize,
}

fn bin_prec(op: &str) -> Option<u8> {
    Some(match op {
        "=" | "<>" | "<" | ">" | "<=" | ">=" => 1,
        "&" => 2,
        "+" | "-" => 3,
        "*" | "/" => 4,
        "^" => 5,
        _ => return None,
    })
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i).map(|t| &t.tok)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.i).map(|t| t.tok.clone());
        self.i += 1;
        t
    }
    fn expr(&mut self, min: u8) -> Result<Expr, String> {
        let mut lhs = self.unary()?;
        loop {
            let Some(Tok::Op(op)) = self.peek().cloned() else { break };
            let Some(prec) = bin_prec(op) else { break };
            if prec < min {
                break;
            }
            self.i += 1;
            // ^ is left-associative in Excel.
            let rhs = self.expr(prec + 1)?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }
    fn unary(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Some(Tok::Op("-")) => {
                self.i += 1;
                let e = self.unary()?;
                Ok(Expr::Neg(Box::new(e)))
            }
            Some(Tok::Op("+")) => {
                self.i += 1;
                self.unary()
            }
            _ => self.postfix(),
        }
    }
    fn postfix(&mut self) -> Result<Expr, String> {
        let mut e = self.primary()?;
        while let Some(Tok::Op("%")) = self.peek() {
            self.i += 1;
            e = Expr::Percent(Box::new(e));
        }
        Ok(e)
    }
    fn primary(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(Expr::Num(n)),
            Some(Tok::Str(s)) => Ok(Expr::Str(s)),
            Some(Tok::ErrLit(e)) => Ok(Expr::Err(e)),
            Some(Tok::LParen) => {
                let e = self.expr(0)?;
                match self.next() {
                    Some(Tok::RParen) => Ok(e),
                    _ => Err(format!("{VALUE} (missing `)`)")),
                }
            }
            Some(Tok::LBrace) => {
                let mut rows = vec![vec![]];
                loop {
                    let e = self.expr(0)?;
                    if let Some(r) = rows.last_mut() {
                        r.push(e);
                    }
                    match self.next() {
                        Some(Tok::Comma) => {}
                        Some(Tok::Semi) => rows.push(vec![]),
                        Some(Tok::RBrace) => break,
                        _ => return Err(format!("{VALUE} (missing `}}`)")),
                    }
                }
                Ok(Expr::Array(rows))
            }
            Some(Tok::Sheet(s)) => match self.next() {
                Some(Tok::Word(w)) => self.reference(Some(s), &w),
                _ => Err(format!("{REF} (expected a cell after {s}!)")),
            },
            Some(Tok::Word(w)) => {
                if let Some(Tok::LParen) = self.peek() {
                    self.i += 1;
                    let mut args = Vec::new();
                    if let Some(Tok::RParen) = self.peek() {
                        self.i += 1;
                        return Ok(Expr::Call(w.to_ascii_uppercase(), args));
                    }
                    loop {
                        // Empty arguments: =IF(A1,,1)
                        if matches!(self.peek(), Some(Tok::Comma) | Some(Tok::Semi)) {
                            args.push(Expr::Name(String::new()));
                        } else {
                            args.push(self.expr(0)?);
                        }
                        match self.next() {
                            Some(Tok::Comma) | Some(Tok::Semi) => {}
                            Some(Tok::RParen) => break,
                            _ => return Err(format!("{VALUE} (missing `)` after {w} arguments)")),
                        }
                    }
                    return Ok(Expr::Call(w.to_ascii_uppercase(), args));
                }
                match w.to_ascii_uppercase().as_str() {
                    "TRUE" => return Ok(Expr::Bool(true)),
                    "FALSE" => return Ok(Expr::Bool(false)),
                    _ => {}
                }
                self.reference(None, &w)
            }
            _ => Err(format!("{VALUE} (incomplete formula)")),
        }
    }
    fn reference(&mut self, sheet: Option<String>, w: &str) -> Result<Expr, String> {
        if let Some(r) = parse_ref_word(w) {
            if let Some(Tok::Colon) = self.peek() {
                self.i += 1;
                let Some(Tok::Word(w2)) = self.next() else { return Err(format!("{REF} (bad range)")) };
                let Some(r2) = parse_ref_word(&w2) else { return Err(format!("{REF} (bad range)")) };
                return Ok(Expr::Range { sheet, r1: r.row.min(r2.row), c1: r.col.min(r2.col), r2: r.row.max(r2.row), c2: r.col.max(r2.col) });
            }
            return Ok(Expr::Ref { sheet, row: r.row, col: r.col });
        }
        if let Some((_, c)) = parse_col_word(w)
            && let Some(Tok::Colon) = self.peek()
        {
            self.i += 1;
            let Some(Tok::Word(w2)) = self.next() else { return Err(format!("{REF} (bad range)")) };
            let Some((_, c2)) = parse_col_word(&w2) else { return Err(format!("{REF} (bad range)")) };
            return Ok(Expr::Range { sheet, r1: 0, c1: c.min(c2), r2: usize::MAX, c2: c.max(c2) });
        }
        if sheet.is_some() {
            return Err(format!("{REF} (bad reference {w})"));
        }
        Ok(Expr::Name(w.to_string()))
    }
}

// =============================================================================================
// Evaluator
// =============================================================================================

struct Book<'a> {
    sheets: Vec<&'a DataSheet>,
    cache: Vec<HashMap<(usize, usize), V>>,
    visiting: Vec<(usize, usize, usize)>,
}

const MAX_DEPTH: usize = 256;

impl<'a> Book<'a> {
    fn new(sheets: Vec<&'a DataSheet>) -> Book<'a> {
        let n = sheets.len();
        Book { sheets, cache: vec![HashMap::new(); n], visiting: Vec::new() }
    }

    fn sheet_index(&self, name: &str) -> Option<usize> {
        self.sheets.iter().position(|s| s.name.eq_ignore_ascii_case(name))
    }

    fn cell(&mut self, si: usize, r: usize, c: usize) -> V {
        if let Some(v) = self.cache.get(si).and_then(|m| m.get(&(r, c))) {
            return v.clone();
        }
        let Some(sheet) = self.sheets.get(si).copied() else { return V::Err(REF) };
        let raw = sheet.cells.get(r).and_then(|row| row.get(c)).map(String::as_str).unwrap_or("");
        let v = match raw.strip_prefix('=') {
            None => input_value(raw),
            Some(f) => {
                if self.visiting.contains(&(si, r, c)) {
                    return V::Err(CIRC);
                }
                if self.visiting.len() > MAX_DEPTH {
                    return V::Err(CIRC);
                }
                self.visiting.push((si, r, c));
                let v = match parse_formula(f) {
                    Ok(e) => self.eval(si, &e).scalar(),
                    Err(e) => V::Err(err_code(&e)),
                };
                self.visiting.pop();
                // A formula pointing at an empty cell shows 0, like Excel.
                if v == V::Empty { V::Num(0.0) } else { v }
            }
        };
        if let Some(m) = self.cache.get_mut(si) {
            m.insert((r, c), v.clone());
        }
        v
    }

    fn range(&mut self, si: usize, r1: usize, c1: usize, r2: usize, c2: usize) -> V {
        let Some(sheet) = self.sheets.get(si).copied() else { return V::Err(REF) };
        let used_rows = sheet.cells.len();
        let r2 = if r2 == usize::MAX { used_rows.saturating_sub(1).max(r1) } else { r2 };
        let rows = r2 - r1 + 1;
        let cols = c2 - c1 + 1;
        if rows.saturating_mul(cols) > 5_000_000 {
            return V::Err(REF);
        }
        let mut data = Vec::with_capacity(rows * cols);
        for r in r1..=r2 {
            for c in c1..=c2 {
                if r >= used_rows {
                    data.push(V::Empty);
                } else {
                    data.push(self.cell(si, r, c));
                }
            }
        }
        V::Arr(Rc::new(Arr { rows, cols, data }))
    }

    fn eval(&mut self, si: usize, e: &Expr) -> V {
        match e {
            Expr::Num(n) => V::Num(*n),
            Expr::Str(s) => V::text(s.as_str()),
            Expr::Bool(b) => V::Bool(*b),
            Expr::Err(e) => V::Err(e),
            Expr::Name(n) => {
                if n.is_empty() {
                    V::Empty
                } else {
                    V::Err(NAME)
                }
            }
            Expr::Ref { sheet, row, col } => {
                let target = match sheet {
                    Some(s) => match self.sheet_index(s) {
                        Some(i) => i,
                        None => return V::Err(REF),
                    },
                    None => si,
                };
                self.cell(target, *row, *col)
            }
            Expr::Range { sheet, r1, c1, r2, c2 } => {
                let target = match sheet {
                    Some(s) => match self.sheet_index(s) {
                        Some(i) => i,
                        None => return V::Err(REF),
                    },
                    None => si,
                };
                self.range(target, *r1, *c1, *r2, *c2)
            }
            Expr::Array(rows) => {
                let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
                let mut data = Vec::new();
                for r in rows {
                    for c in 0..cols {
                        data.push(r.get(c).map(|x| self.eval(si, x).scalar()).unwrap_or(V::Err(NA)));
                    }
                }
                V::Arr(Rc::new(Arr { rows: rows.len(), cols, data }))
            }
            Expr::Neg(x) => {
                let v = self.eval(si, x);
                map1(&v, |a| a.num().map(|n| V::Num(-n)).unwrap_or_else(V::Err))
            }
            Expr::Percent(x) => {
                let v = self.eval(si, x);
                map1(&v, |a| a.num().map(|n| V::Num(n / 100.0)).unwrap_or_else(V::Err))
            }
            Expr::Bin(op, a, b) => {
                let va = self.eval(si, a);
                let vb = self.eval(si, b);
                map2(&va, &vb, |x, y| binop(op, x, y))
            }
            Expr::Call(name, args) => self.call(si, name, args),
        }
    }

    fn args(&mut self, si: usize, args: &[Expr]) -> Vec<V> {
        args.iter().map(|a| self.eval(si, a)).collect()
    }

    fn call(&mut self, si: usize, name: &str, args: &[Expr]) -> V {
        let n = args.len();
        let need = |lo: usize, hi: usize| n >= lo && n <= hi;
        macro_rules! arity {
            ($lo:expr, $hi:expr) => {
                if !need($lo, $hi) {
                    return V::Err(VALUE);
                }
            };
        }
        macro_rules! tryv {
            ($e:expr) => {
                match $e {
                    Ok(x) => x,
                    Err(e) => return V::Err(e),
                }
            };
        }
        // Lazily evaluated functions first.
        match name {
            "IF" => {
                arity!(1, 3);
                let c = self.eval(si, &args[0]);
                if let V::Arr(_) = c {
                    let t = args.get(1).map(|a| self.eval(si, a)).unwrap_or(V::Bool(true));
                    let f = args.get(2).map(|a| self.eval(si, a)).unwrap_or(V::Bool(false));
                    return map2(&c, &t, |cc, tt| match cc.bool() {
                        Ok(true) => tt.clone(),
                        Ok(false) => f.clone().scalar(),
                        Err(e) => V::Err(e),
                    });
                }
                return match c.bool() {
                    Ok(true) => args.get(1).map(|a| self.eval(si, a)).unwrap_or(V::Bool(true)),
                    Ok(false) => args.get(2).map(|a| self.eval(si, a)).unwrap_or(V::Bool(false)),
                    Err(e) => V::Err(e),
                };
            }
            "IFS" => {
                if n < 2 || n % 2 != 0 {
                    return V::Err(VALUE);
                }
                for pair in args.chunks(2) {
                    let (Some(c), Some(v)) = (pair.first(), pair.get(1)) else { break };
                    match self.eval(si, c).scalar().bool() {
                        Ok(true) => return self.eval(si, v),
                        Ok(false) => {}
                        Err(e) => return V::Err(e),
                    }
                }
                return V::Err(NA);
            }
            "IFERROR" | "IFNA" => {
                arity!(2, 2);
                let v = self.eval(si, &args[0]);
                let bad = match &v.clone().scalar() {
                    V::Err(e) => name == "IFERROR" || *e == NA,
                    _ => false,
                };
                return if bad { self.eval(si, &args[1]) } else { v };
            }
            "SWITCH" => {
                if n < 3 {
                    return V::Err(VALUE);
                }
                let x = self.eval(si, &args[0]).scalar();
                let mut i = 1;
                while i + 1 < n {
                    let k = self.eval(si, &args[i]).scalar();
                    if cmp_values(&x, &k) == Some(std::cmp::Ordering::Equal) {
                        return self.eval(si, &args[i + 1]);
                    }
                    i += 2;
                }
                return if i < n { self.eval(si, &args[i]) } else { V::Err(NA) };
            }
            "CHOOSE" => {
                if n < 2 {
                    return V::Err(VALUE);
                }
                let k = tryv!(self.eval(si, &args[0]).num()) as usize;
                return match args.get(k) {
                    Some(a) if k >= 1 => self.eval(si, a),
                    _ => V::Err(VALUE),
                };
            }
            _ => {}
        }
        let a = self.args(si, args);
        let arg = |i: usize| a.get(i).cloned().unwrap_or(V::Empty);
        let num = |i: usize| a.get(i).cloned().unwrap_or(V::Empty).scalar().num();
        let txt = |i: usize| a.get(i).cloned().unwrap_or(V::Empty).scalar().str();
        match name {
            // ---- aggregates
            "SUM" => agg(&a, |xs| Ok(xs.iter().sum())),
            "AVERAGE" => agg(&a, |xs| if xs.is_empty() { Err(DIV0) } else { Ok(xs.iter().sum::<f64>() / xs.len() as f64) }),
            "MIN" => agg(&a, |xs| Ok(if xs.is_empty() { 0.0 } else { xs.iter().copied().fold(f64::INFINITY, f64::min) })),
            "MAX" => agg(&a, |xs| Ok(if xs.is_empty() { 0.0 } else { xs.iter().copied().fold(f64::NEG_INFINITY, f64::max) })),
            "PRODUCT" => agg(&a, |xs| Ok(xs.iter().product())),
            "MEDIAN" => agg(&a, |xs| {
                if xs.is_empty() {
                    return Err(NUM);
                }
                let mut v = xs.to_vec();
                v.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
                let m = v.len() / 2;
                Ok(if v.len() % 2 == 0 { (v.get(m - 1).copied().unwrap_or(0.0) + v.get(m).copied().unwrap_or(0.0)) / 2.0 } else { v.get(m).copied().unwrap_or(0.0) })
            }),
            "STDEV" | "VAR" => agg(&a, |xs| {
                if xs.len() < 2 {
                    return Err(DIV0);
                }
                let m = xs.iter().sum::<f64>() / xs.len() as f64;
                let var = xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (xs.len() - 1) as f64;
                Ok(if name == "VAR" { var } else { var.sqrt() })
            }),
            "COUNT" => V::Num(a.iter().flat_map(|v| match v {
                V::Arr(_) => v.flat().into_iter().filter(|x| matches!(x, V::Num(_))).map(|_| 1).collect::<Vec<_>>(),
                x => if x.num().is_ok() && !matches!(x, V::Empty) { vec![1] } else { vec![] },
            }).count() as f64),
            "COUNTA" => V::Num(a.iter().flat_map(|v| v.flat()).filter(|x| !matches!(x, V::Empty)).count() as f64),
            "COUNTBLANK" => V::Num(a.iter().flat_map(|v| v.flat()).filter(|x| matches!(x, V::Empty) || matches!(x, V::Text(t) if t.is_empty())).count() as f64),
            "SUMPRODUCT" => {
                if a.is_empty() {
                    return V::Err(VALUE);
                }
                let dims = a.first().map(|v| v.dims()).unwrap_or((1, 1));
                if a.iter().any(|v| v.dims() != dims) {
                    return V::Err(VALUE);
                }
                let flats: Vec<Vec<V>> = a.iter().map(|v| v.flat()).collect();
                let len = dims.0 * dims.1;
                let mut total = 0.0;
                for i in 0..len {
                    let mut p = 1.0;
                    for f in &flats {
                        match f.get(i) {
                            Some(V::Num(x)) => p *= x,
                            Some(V::Bool(b)) => p *= if *b { 1.0 } else { 0.0 },
                            Some(V::Err(e)) => return V::Err(e),
                            _ => p = 0.0,
                        }
                    }
                    total += p;
                }
                V::Num(total)
            }
            "SUMIF" | "AVERAGEIF" | "COUNTIF" => {
                let lo = if name == "COUNTIF" { 2 } else { 2 };
                arity!(lo, if name == "COUNTIF" { 2 } else { 3 });
                let range = arg(0).flat();
                let crit = Criteria::new(&arg(1).scalar());
                let sum_range = if n == 3 { arg(2).flat() } else { range.clone() };
                let mut total = 0.0;
                let mut count = 0usize;
                for (i, v) in range.iter().enumerate() {
                    if crit.matches(v) {
                        count += 1;
                        if let Some(V::Num(x)) = sum_range.get(i) {
                            total += x;
                        }
                    }
                }
                match name {
                    "COUNTIF" => V::Num(count as f64),
                    "SUMIF" => V::Num(total),
                    _ => {
                        let nums = range.iter().enumerate().filter(|(i, v)| crit.matches(v) && matches!(sum_range.get(*i), Some(V::Num(_)))).count();
                        if nums == 0 { V::Err(DIV0) } else { V::Num(total / nums as f64) }
                    }
                }
            }
            "SUMIFS" | "COUNTIFS" => {
                let (sum_range, pairs) = if name == "SUMIFS" { (Some(arg(0).flat()), a.get(1..).unwrap_or(&[])) } else { (None, a.as_slice()) };
                if pairs.is_empty() || pairs.len() % 2 != 0 {
                    return V::Err(VALUE);
                }
                let conds: Vec<(Vec<V>, Criteria)> = pairs.chunks(2).map(|p| (p.first().map(|x| x.flat()).unwrap_or_default(), Criteria::new(&p.get(1).cloned().unwrap_or(V::Empty).scalar()))).collect();
                let len = conds.first().map(|c| c.0.len()).unwrap_or(0);
                let mut total = 0.0;
                let mut count = 0;
                for i in 0..len {
                    if conds.iter().all(|(r, c)| r.get(i).is_some_and(|v| c.matches(v))) {
                        count += 1;
                        if let Some(sr) = &sum_range
                            && let Some(V::Num(x)) = sr.get(i)
                        {
                            total += x;
                        }
                    }
                }
                V::Num(if sum_range.is_some() { total } else { count as f64 })
            }
            // ---- logic
            "AND" | "OR" => {
                let mut vals = Vec::new();
                for v in &a {
                    for x in v.flat() {
                        match x {
                            V::Empty => {}
                            V::Text(_) if matches!(v, V::Arr(_)) => {}
                            other => vals.push(tryv!(other.bool())),
                        }
                    }
                }
                if vals.is_empty() {
                    return V::Err(VALUE);
                }
                V::Bool(if name == "AND" { vals.iter().all(|b| *b) } else { vals.iter().any(|b| *b) })
            }
            "NOT" => {
                arity!(1, 1);
                map1(&arg(0), |x| x.bool().map(|b| V::Bool(!b)).unwrap_or_else(V::Err))
            }
            "TRUE" => V::Bool(true),
            "FALSE" => V::Bool(false),
            "ISBLANK" => V::Bool(matches!(arg(0).scalar(), V::Empty)),
            "ISNUMBER" => V::Bool(matches!(arg(0).scalar(), V::Num(_))),
            "ISTEXT" => V::Bool(matches!(arg(0).scalar(), V::Text(_))),
            "ISERROR" => V::Bool(arg(0).scalar().is_err()),
            // ---- math
            "ROUND" | "ROUNDUP" | "ROUNDDOWN" => {
                arity!(1, 2);
                let x = tryv!(num(0));
                let d = if n > 1 { tryv!(num(1)) as i32 } else { 0 };
                let f = 10f64.powi(d);
                let y = x * f;
                let r = match name {
                    "ROUND" => {
                        // Round half away from zero, guarding binary representation issues.
                        let y2 = (y * 1e9).round() / 1e9;
                        y2.round()
                    }
                    "ROUNDUP" => {
                        if y >= 0.0 { (y - 1e-9).ceil() } else { (y + 1e-9).floor() }
                    }
                    _ => y.trunc(),
                };
                V::Num(r / f)
            }
            "CEILING" | "FLOOR" => {
                arity!(1, 2);
                let x = tryv!(num(0));
                let s = if n > 1 { tryv!(num(1)) } else { 1.0 };
                if s == 0.0 {
                    return V::Num(0.0);
                }
                V::Num(if name == "CEILING" { (x / s).ceil() * s } else { (x / s).floor() * s })
            }
            "ABS" => math1(&a, f64::abs),
            "SQRT" => {
                arity!(1, 1);
                let x = tryv!(num(0));
                if x < 0.0 { V::Err(NUM) } else { V::Num(x.sqrt()) }
            }
            "EXP" => math1(&a, f64::exp),
            "LN" | "LOG10" | "LOG" => {
                arity!(1, 2);
                let x = tryv!(num(0));
                if x <= 0.0 {
                    return V::Err(NUM);
                }
                let base = if name == "LN" {
                    std::f64::consts::E
                } else if name == "LOG10" {
                    10.0
                } else if n > 1 {
                    tryv!(num(1))
                } else {
                    10.0
                };
                V::Num(x.ln() / base.ln())
            }
            "SIGN" => math1(&a, |x| if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 }),
            "INT" => math1(&a, f64::floor),
            "PI" => V::Num(std::f64::consts::PI),
            "RAND" => V::Num(0.5),
            "POWER" => {
                arity!(2, 2);
                let r = tryv!(num(0)).powf(tryv!(num(1)));
                if r.is_finite() { V::Num(r) } else { V::Err(NUM) }
            }
            "MOD" => {
                arity!(2, 2);
                let x = tryv!(num(0));
                let d = tryv!(num(1));
                if d == 0.0 { V::Err(DIV0) } else { V::Num(x - d * (x / d).floor()) }
            }
            // ---- text
            "LEN" => {
                arity!(1, 1);
                map1(&arg(0), |x| x.str().map(|s| V::Num(s.chars().count() as f64)).unwrap_or_else(V::Err))
            }
            "LEFT" | "RIGHT" => {
                arity!(1, 2);
                let s = tryv!(txt(0));
                let k = if n > 1 { tryv!(num(1)) } else { 1.0 };
                if k < 0.0 {
                    return V::Err(VALUE);
                }
                let chars: Vec<char> = s.chars().collect();
                let k = (k as usize).min(chars.len());
                let part: String = if name == "LEFT" { chars.iter().take(k).collect() } else { chars.iter().skip(chars.len() - k).collect() };
                V::text(part)
            }
            "MID" => {
                arity!(3, 3);
                let s = tryv!(txt(0));
                let start = tryv!(num(1));
                let len = tryv!(num(2));
                if start < 1.0 || len < 0.0 {
                    return V::Err(VALUE);
                }
                V::text(s.chars().skip(start as usize - 1).take(len as usize).collect::<String>())
            }
            "UPPER" => map1(&arg(0), |x| x.str().map(|s| V::text(s.to_uppercase())).unwrap_or_else(V::Err)),
            "LOWER" => map1(&arg(0), |x| x.str().map(|s| V::text(s.to_lowercase())).unwrap_or_else(V::Err)),
            "PROPER" => {
                let s = tryv!(txt(0));
                let mut out = String::new();
                let mut prev_alpha = false;
                for ch in s.chars() {
                    if prev_alpha { out.extend(ch.to_lowercase()) } else { out.extend(ch.to_uppercase()) }
                    prev_alpha = ch.is_alphabetic();
                }
                V::text(out)
            }
            "TRIM" => {
                arity!(1, 1);
                map1(&arg(0), |x| x.str().map(|s| V::text(s.split_whitespace().collect::<Vec<_>>().join(" "))).unwrap_or_else(V::Err))
            }
            "CONCAT" | "CONCATENATE" => {
                let mut s = String::new();
                for v in &a {
                    for x in v.flat() {
                        s.push_str(&tryv!(x.str()));
                    }
                }
                V::text(s)
            }
            "TEXTJOIN" => {
                if n < 3 {
                    return V::Err(VALUE);
                }
                let sep = tryv!(txt(0));
                let skip_empty = tryv!(arg(1).scalar().bool());
                let mut parts = Vec::new();
                for v in a.iter().skip(2) {
                    for x in v.flat() {
                        let s = tryv!(x.str());
                        if !(skip_empty && s.is_empty()) {
                            parts.push(s);
                        }
                    }
                }
                V::text(parts.join(&sep))
            }
            "REPT" => {
                let s = tryv!(txt(0));
                let k = tryv!(num(1)).max(0.0) as usize;
                if s.len().saturating_mul(k) > 32_767 {
                    return V::Err(VALUE);
                }
                V::text(s.repeat(k))
            }
            "SUBSTITUTE" => {
                arity!(3, 4);
                let s = tryv!(txt(0));
                let old = tryv!(txt(1));
                let new = tryv!(txt(2));
                if old.is_empty() {
                    return V::text(s);
                }
                if n == 4 {
                    let k = tryv!(num(3)) as usize;
                    let mut count = 0;
                    let mut out = String::new();
                    let mut rest = s.as_str();
                    while let Some(p) = rest.find(&old) {
                        count += 1;
                        out.push_str(rest.get(..p).unwrap_or(""));
                        out.push_str(if count == k { &new } else { &old });
                        rest = rest.get(p + old.len()..).unwrap_or("");
                    }
                    out.push_str(rest);
                    V::text(out)
                } else {
                    V::text(s.replace(&old, &new))
                }
            }
            "FIND" | "SEARCH" => {
                arity!(2, 3);
                let needle = tryv!(txt(0));
                let hay = tryv!(txt(1));
                let start = if n > 2 { tryv!(num(2)) as usize } else { 1 };
                if start < 1 {
                    return V::Err(VALUE);
                }
                let (h, nd) = if name == "SEARCH" { (hay.to_lowercase(), needle.to_lowercase()) } else { (hay, needle) };
                let hc: Vec<char> = h.chars().collect();
                let nc: Vec<char> = nd.chars().collect();
                let from = start - 1;
                if nc.is_empty() {
                    return V::Num(start as f64);
                }
                for i in from..hc.len() {
                    if hc.get(i..i + nc.len()).is_some_and(|w| w == nc.as_slice()) {
                        return V::Num((i + 1) as f64);
                    }
                }
                V::Err(VALUE)
            }
            "EXACT" => V::Bool(tryv!(txt(0)) == tryv!(txt(1))),
            "CHAR" => V::text(char::from_u32(tryv!(num(0)) as u32).map(|c| c.to_string()).unwrap_or_default()),
            "VALUE" => {
                arity!(1, 1);
                match arg(0).scalar() {
                    V::Num(x) => V::Num(x),
                    V::Text(t) => text_to_num(&t).map(V::Num).unwrap_or(V::Err(VALUE)),
                    V::Empty => V::Num(0.0),
                    V::Bool(_) => V::Err(VALUE),
                    other => other,
                }
            }
            "TEXT" => {
                arity!(2, 2);
                let f = tryv!(txt(1));
                match arg(0).scalar() {
                    V::Text(t) => match text_to_num(&t) {
                        Some(x) => V::text(CellValue::Num(x).display(&f)),
                        None => V::Text(t),
                    },
                    other => V::text(CellValue::Num(tryv!(other.num())).display(&f)),
                }
            }
            // ---- dates
            "TODAY" | "NOW" => {
                let now = chrono::Local::now().naive_local();
                let d = date_serial(now.date());
                if name == "TODAY" {
                    V::Num(d)
                } else {
                    use chrono::Timelike;
                    V::Num(d + now.time().num_seconds_from_midnight() as f64 / 86400.0)
                }
            }
            "DATE" => {
                arity!(3, 3);
                let y = tryv!(num(0)) as i32;
                let m = tryv!(num(1)) as i32;
                let d = tryv!(num(2)) as i64;
                make_date(y, m, d).map(V::Num).unwrap_or(V::Err(NUM))
            }
            "YEAR" | "MONTH" | "DAY" | "WEEKDAY" => {
                arity!(1, 2);
                let Some(date) = serial_date(tryv!(num(0))) else { return V::Err(NUM) };
                use chrono::Datelike;
                V::Num(match name {
                    "YEAR" => date.year() as f64,
                    "MONTH" => date.month() as f64,
                    "DAY" => date.day() as f64,
                    _ => {
                        let ty = if n > 1 { tryv!(num(1)) as i32 } else { 1 };
                        let sun0 = date.weekday().num_days_from_sunday() as f64; // Sun = 0
                        match ty {
                            2 => date.weekday().num_days_from_monday() as f64 + 1.0,
                            3 => date.weekday().num_days_from_monday() as f64,
                            _ => sun0 + 1.0,
                        }
                    }
                })
            }
            "HOUR" | "MINUTE" | "SECOND" => {
                let x = tryv!(num(0));
                let secs = ((x - x.floor()) * 86400.0).round() as i64;
                V::Num(match name {
                    "HOUR" => (secs / 3600) as f64,
                    "MINUTE" => ((secs / 60) % 60) as f64,
                    _ => (secs % 60) as f64,
                })
            }
            "EOMONTH" | "EDATE" => {
                arity!(2, 2);
                let Some(date) = serial_date(tryv!(num(0))) else { return V::Err(NUM) };
                let months = tryv!(num(1)) as i32;
                use chrono::Datelike;
                let total = date.year() * 12 + date.month0() as i32 + months;
                let (y, m) = (total.div_euclid(12), total.rem_euclid(12) + 1);
                if name == "EOMONTH" {
                    make_date(y, m + 1, 0).map(V::Num).unwrap_or(V::Err(NUM))
                } else {
                    let last = make_date(y, m + 1, 0).and_then(serial_date).map(|d| d.day()).unwrap_or(28);
                    make_date(y, m, date.day().min(last) as i64).map(V::Num).unwrap_or(V::Err(NUM))
                }
            }
            "DAYS" => V::Num((tryv!(num(0)).floor() - tryv!(num(1)).floor()).trunc()),
            // ---- lookup
            "VLOOKUP" | "HLOOKUP" => {
                arity!(3, 4);
                let key = arg(0).scalar();
                if let V::Err(e) = key {
                    return V::Err(e);
                }
                let V::Arr(t) = arg(1) else { return V::Err(NA) };
                let idx = tryv!(num(2)) as usize;
                let approx = if n > 3 { tryv!(arg(3).scalar().bool()) } else { true };
                let vertical = name == "VLOOKUP";
                let (len, width) = if vertical { (t.rows, t.cols) } else { (t.cols, t.rows) };
                if idx < 1 || idx > width {
                    return V::Err(REF);
                }
                let at = |i: usize, j: usize| if vertical { t.get(i, j) } else { t.get(j, i) };
                let mut found = None;
                if approx {
                    for i in 0..len {
                        match cmp_values(&at(i, 0), &key) {
                            Some(std::cmp::Ordering::Greater) => break,
                            Some(_) => found = Some(i),
                            None => {}
                        }
                    }
                } else {
                    let crit = Criteria::equals(&key);
                    found = (0..len).find(|i| crit.matches(&at(*i, 0)));
                }
                match found {
                    Some(i) => at(i, idx - 1),
                    None => V::Err(NA),
                }
            }
            "XLOOKUP" => {
                arity!(3, 6);
                let key = arg(0).scalar();
                let look = arg(1).flat();
                let ret = arg(2);
                let mode = if n > 4 { tryv!(num(4)) as i32 } else { 0 };
                let search = if n > 5 { tryv!(num(5)) as i32 } else { 1 };
                let order: Vec<usize> = if search < 0 { (0..look.len()).rev().collect() } else { (0..look.len()).collect() };
                let crit = if mode == 2 { Criteria::new(&key) } else { Criteria::equals(&key) };
                let mut found = order.iter().copied().find(|i| look.get(*i).is_some_and(|v| crit.matches(v)));
                if found.is_none() && (mode == -1 || mode == 1) {
                    // Next smaller (-1) / next larger (1).
                    let mut best: Option<(usize, V)> = None;
                    for (i, v) in look.iter().enumerate() {
                        let ord = cmp_values(v, &key);
                        let ok = if mode == -1 { ord == Some(std::cmp::Ordering::Less) } else { ord == Some(std::cmp::Ordering::Greater) };
                        if ok {
                            let better = match &best {
                                None => true,
                                Some((_, bv)) => {
                                    let o = cmp_values(v, bv);
                                    if mode == -1 { o == Some(std::cmp::Ordering::Greater) } else { o == Some(std::cmp::Ordering::Less) }
                                }
                            };
                            if better {
                                best = Some((i, v.clone()));
                            }
                        }
                    }
                    found = best.map(|b| b.0);
                }
                match found {
                    Some(i) => match &ret {
                        V::Arr(r) => {
                            let (lr, _) = arg(1).dims();
                            if r.cols > 1 && lr > 1 {
                                // Return the whole row.
                                let data: Vec<V> = (0..r.cols).map(|c| r.get(i, c)).collect();
                                V::Arr(Rc::new(Arr { rows: 1, cols: r.cols, data }))
                            } else {
                                r.data.get(i).cloned().unwrap_or(V::Err(NA))
                            }
                        }
                        other => other.clone(),
                    },
                    None => {
                        if n > 3 && !matches!(args.get(3), Some(Expr::Name(s)) if s.is_empty()) {
                            arg(3)
                        } else {
                            V::Err(NA)
                        }
                    }
                }
            }
            "INDEX" => {
                arity!(2, 3);
                let src = arg(0);
                let r = tryv!(num(1)) as usize;
                let c = if n > 2 { tryv!(num(2)) as usize } else { 0 };
                match &src {
                    V::Arr(t) => {
                        let (r, c) = if t.rows == 1 && n == 2 { (1, r) } else { (r, c.max(if t.cols == 1 { 1 } else { c })) };
                        if r == 0 && c >= 1 && c <= t.cols {
                            let data: Vec<V> = (0..t.rows).map(|i| t.get(i, c - 1)).collect();
                            return V::Arr(Rc::new(Arr { rows: t.rows, cols: 1, data }));
                        }
                        if c == 0 && r >= 1 && r <= t.rows {
                            let data: Vec<V> = (0..t.cols).map(|j| t.get(r - 1, j)).collect();
                            return V::Arr(Rc::new(Arr { rows: 1, cols: t.cols, data }));
                        }
                        if r < 1 || c < 1 || r > t.rows || c > t.cols {
                            return V::Err(REF);
                        }
                        t.get(r - 1, c - 1)
                    }
                    other if r <= 1 && c <= 1 => other.clone(),
                    _ => V::Err(REF),
                }
            }
            "MATCH" => {
                arity!(2, 3);
                let key = arg(0).scalar();
                let look = arg(1).flat();
                let ty = if n > 2 { tryv!(num(2)) as i32 } else { 1 };
                let found = match ty {
                    0 => {
                        let crit = Criteria::new(&key);
                        let crit = if matches!(key, V::Text(_)) { crit } else { Criteria::equals(&key) };
                        look.iter().position(|v| crit.matches(v))
                    }
                    1 => {
                        let mut f = None;
                        for (i, v) in look.iter().enumerate() {
                            match cmp_values(v, &key) {
                                Some(std::cmp::Ordering::Greater) => break,
                                Some(_) => f = Some(i),
                                None => {}
                            }
                        }
                        f
                    }
                    _ => {
                        let mut f = None;
                        for (i, v) in look.iter().enumerate() {
                            match cmp_values(v, &key) {
                                Some(std::cmp::Ordering::Less) => break,
                                Some(_) => f = Some(i),
                                None => {}
                            }
                        }
                        f
                    }
                };
                found.map(|i| V::Num((i + 1) as f64)).unwrap_or(V::Err(NA))
            }
            _ => V::Err(NAME),
        }
    }
}

fn err_code(msg: &str) -> &'static str {
    for e in [DIV0, REF, NAME, VALUE, NA, NUM] {
        if msg.starts_with(e) {
            return e;
        }
    }
    VALUE
}

fn map1(v: &V, f: impl Fn(&V) -> V) -> V {
    match v {
        V::Arr(a) => V::Arr(Rc::new(Arr { rows: a.rows, cols: a.cols, data: a.data.iter().map(&f).collect() })),
        x => f(x),
    }
}

/// Element-wise with broadcasting of single values / rows / columns.
fn map2(a: &V, b: &V, f: impl Fn(&V, &V) -> V) -> V {
    match (a, b) {
        (V::Arr(x), V::Arr(y)) => {
            let rows = x.rows.max(y.rows);
            let cols = x.cols.max(y.cols);
            let pick = |t: &Arr, r: usize, c: usize| {
                let rr = if t.rows == 1 { 0 } else { r };
                let cc = if t.cols == 1 { 0 } else { c };
                if rr >= t.rows || cc >= t.cols { V::Err(NA) } else { t.get(rr, cc) }
            };
            let mut data = Vec::with_capacity(rows * cols);
            for r in 0..rows {
                for c in 0..cols {
                    data.push(f(&pick(x, r, c), &pick(y, r, c)));
                }
            }
            V::Arr(Rc::new(Arr { rows, cols, data }))
        }
        (V::Arr(x), s) => V::Arr(Rc::new(Arr { rows: x.rows, cols: x.cols, data: x.data.iter().map(|v| f(v, s)).collect() })),
        (s, V::Arr(y)) => V::Arr(Rc::new(Arr { rows: y.rows, cols: y.cols, data: y.data.iter().map(|v| f(s, v)).collect() })),
        (x, y) => f(x, y),
    }
}

fn binop(op: &str, a: &V, b: &V) -> V {
    if let V::Err(e) = a {
        return V::Err(e);
    }
    if let V::Err(e) = b {
        return V::Err(e);
    }
    match op {
        "&" => match (a.str(), b.str()) {
            (Ok(x), Ok(y)) => V::text(x + &y),
            (Err(e), _) | (_, Err(e)) => V::Err(e),
        },
        "=" | "<>" | "<" | ">" | "<=" | ">=" => {
            use std::cmp::Ordering::*;
            let ord = cmp_values(a, b).unwrap_or(Equal);
            V::Bool(match op {
                "=" => ord == Equal,
                "<>" => ord != Equal,
                "<" => ord == Less,
                ">" => ord == Greater,
                "<=" => ord != Greater,
                _ => ord != Less,
            })
        }
        _ => {
            let (x, y) = match (a.num(), b.num()) {
                (Ok(x), Ok(y)) => (x, y),
                (Err(e), _) | (_, Err(e)) => return V::Err(e),
            };
            let r = match op {
                "+" => x + y,
                "-" => x - y,
                "*" => x * y,
                "/" => {
                    if y == 0.0 {
                        return V::Err(DIV0);
                    }
                    x / y
                }
                "^" => {
                    let r = x.powf(y);
                    if !r.is_finite() {
                        return V::Err(NUM);
                    }
                    r
                }
                _ => return V::Err(VALUE),
            };
            V::Num(r)
        }
    }
}

/// Excel ordering: numbers < text < booleans; text compares case-insensitively; empty is 0 or "".
fn cmp_values(a: &V, b: &V) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    let rank = |v: &V| match v {
        V::Num(_) => 0,
        V::Text(_) => 1,
        V::Bool(_) => 2,
        _ => 3,
    };
    match (a, b) {
        (V::Empty, V::Empty) => Some(Ordering::Equal),
        (V::Empty, V::Num(y)) => 0.0f64.partial_cmp(y),
        (V::Num(x), V::Empty) => x.partial_cmp(&0.0),
        (V::Empty, V::Text(t)) => Some(if t.is_empty() { Ordering::Equal } else { Ordering::Less }),
        (V::Text(t), V::Empty) => Some(if t.is_empty() { Ordering::Equal } else { Ordering::Greater }),
        (V::Empty, V::Bool(y)) => Some(false.cmp(y)),
        (V::Bool(x), V::Empty) => Some(x.cmp(&false)),
        (V::Num(x), V::Num(y)) => x.partial_cmp(y),
        (V::Text(x), V::Text(y)) => Some(x.to_lowercase().cmp(&y.to_lowercase())),
        (V::Bool(x), V::Bool(y)) => Some(x.cmp(y)),
        (V::Err(_), _) | (_, V::Err(_)) => None,
        (V::Arr(x), y) => cmp_values(&x.data.first().cloned().unwrap_or(V::Empty), y),
        (x, V::Arr(y)) => cmp_values(x, &y.data.first().cloned().unwrap_or(V::Empty)),
        (x, y) => Some(rank(x).cmp(&rank(y))),
    }
}

/// Numbers of the arguments: from ranges only numbers count; direct arguments are coerced.
fn agg(args: &[V], f: impl Fn(&[f64]) -> Result<f64, &'static str>) -> V {
    let mut xs = Vec::new();
    for a in args {
        match a {
            V::Arr(arr) => {
                for v in &arr.data {
                    match v {
                        V::Num(n) => xs.push(*n),
                        V::Err(e) => return V::Err(e),
                        _ => {}
                    }
                }
            }
            V::Empty => {}
            v => match v.num() {
                Ok(n) => xs.push(n),
                Err(e) => return V::Err(e),
            },
        }
    }
    match f(&xs) {
        Ok(n) => V::Num(n),
        Err(e) => V::Err(e),
    }
}

fn math1(args: &[V], f: impl Fn(f64) -> f64) -> V {
    if args.len() != 1 {
        return V::Err(VALUE);
    }
    map1(args.first().unwrap_or(&V::Empty), |x| match x.num() {
        Ok(n) => {
            let r = f(n);
            if r.is_finite() { V::Num(r) } else { V::Err(NUM) }
        }
        Err(e) => V::Err(e),
    })
}

fn date_base() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(1899, 12, 30).unwrap_or_default()
}

fn date_serial(d: chrono::NaiveDate) -> f64 {
    (d - date_base()).num_days() as f64
}

fn serial_date(s: f64) -> Option<chrono::NaiveDate> {
    if !s.is_finite() || s.abs() > 3_000_000.0 {
        return None;
    }
    date_base().checked_add_signed(chrono::Duration::days(s.floor() as i64))
}

/// DATE(y, m, d) with Excel's overflow rules (month 13 = January next year, day 0 = last day
/// of the previous month).
fn make_date(y: i32, m: i32, d: i64) -> Option<f64> {
    let y = if (0..1900).contains(&y) { y + 1900 } else { y };
    let total = y * 12 + (m - 1);
    let (yy, mm) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    let first = chrono::NaiveDate::from_ymd_opt(yy, mm as u32, 1)?;
    let date = first.checked_add_signed(chrono::Duration::days(d - 1))?;
    Some(date_serial(date))
}

// ---------------------------------------------------------------------------------------------
// Criteria (SUMIF / COUNTIF / MATCH with wildcards)
// ---------------------------------------------------------------------------------------------

enum Criteria {
    Cmp(&'static str, V),
    Pattern(bool, Vec<char>),
}

impl Criteria {
    fn equals(v: &V) -> Criteria {
        Criteria::Cmp("=", v.clone())
    }
    fn new(v: &V) -> Criteria {
        match v {
            V::Text(t) => {
                let s = t.as_ref();
                let (op, rest) = if let Some(r) = s.strip_prefix(">=") {
                    (">=", r)
                } else if let Some(r) = s.strip_prefix("<=") {
                    ("<=", r)
                } else if let Some(r) = s.strip_prefix("<>") {
                    ("<>", r)
                } else if let Some(r) = s.strip_prefix('>') {
                    (">", r)
                } else if let Some(r) = s.strip_prefix('<') {
                    ("<", r)
                } else if let Some(r) = s.strip_prefix('=') {
                    ("=", r)
                } else {
                    ("=", s)
                };
                let target = input_value(rest);
                if let V::Text(tt) = &target
                    && (op == "=" || op == "<>")
                    && (tt.contains('*') || tt.contains('?'))
                {
                    return Criteria::Pattern(op == "=", tt.to_lowercase().chars().collect());
                }
                Criteria::Cmp(op, if rest.is_empty() { V::Empty } else { target })
            }
            other => Criteria::Cmp("=", other.clone()),
        }
    }
    fn matches(&self, v: &V) -> bool {
        match self {
            Criteria::Pattern(eq, p) => {
                let s: Vec<char> = match v {
                    V::Text(t) => t.to_lowercase().chars().collect(),
                    V::Num(n) => crate::numfmt::general(*n).chars().collect(),
                    _ => vec![],
                };
                wildcard(p, &s) == *eq
            }
            Criteria::Cmp(op, target) => {
                if matches!(target, V::Empty) {
                    let blank = matches!(v, V::Empty) || matches!(v, V::Text(t) if t.is_empty());
                    return if *op == "<>" { !blank } else { blank };
                }
                // Numbers only compare with numbers (and number-like text), text with text.
                let same_kind = match (v, target) {
                    (V::Num(_), V::Num(_)) | (V::Text(_), V::Text(_)) | (V::Bool(_), V::Bool(_)) => true,
                    (V::Text(t), V::Num(_)) => parse_number(t).is_some(),
                    _ => false,
                };
                let vv = match (v, target) {
                    (V::Text(t), V::Num(_)) => parse_number(t).map(V::Num).unwrap_or(V::Empty),
                    _ => v.clone(),
                };
                if !same_kind {
                    return *op == "<>";
                }
                let Some(ord) = cmp_values(&vv, target) else { return false };
                use std::cmp::Ordering::*;
                match *op {
                    "=" => ord == Equal,
                    "<>" => ord != Equal,
                    ">" => ord == Greater,
                    "<" => ord == Less,
                    ">=" => ord != Less,
                    _ => ord != Greater,
                }
            }
        }
    }
}

fn wildcard(p: &[char], s: &[char]) -> bool {
    // Iterative glob matching with * and ? (and ~ escapes).
    let (mut pi, mut si) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while si < s.len() {
        match p.get(pi) {
            Some('*') => {
                star = Some((pi, si));
                pi += 1;
            }
            Some('?') => {
                pi += 1;
                si += 1;
            }
            Some('~') if p.get(pi + 1).is_some_and(|c| Some(c) == s.get(si)) => {
                pi += 2;
                si += 1;
            }
            Some(c) if Some(c) == s.get(si) => {
                pi += 1;
                si += 1;
            }
            _ => match star {
                Some((sp, ss)) => {
                    pi = sp + 1;
                    si = ss + 1;
                    star = Some((sp, ss + 1));
                }
                None => return false,
            },
        }
    }
    while p.get(pi) == Some(&'*') {
        pi += 1;
    }
    pi == p.len()
}

// =============================================================================================
// Tests
// =============================================================================================

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn sheet(name: &str, rows: &[&[&str]]) -> DataSheet {
        DataSheet { name: name.into(), cells: rows.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect(), load: true, ..DataSheet::default() }
    }

    fn eval1(rows: &[&[&str]], f: &str) -> CellValue {
        let s = sheet("S", rows);
        evaluate_formula(&s, &[], f)
    }

    fn num(v: CellValue) -> f64 {
        match v {
            CellValue::Num(n) => n,
            other => panic!("not a number: {other:?}"),
        }
    }

    const DATA: &[&[&str]] = &[&["Name", "Qty", "Price"], &["apple", "3", "1.5"], &["pear", "5", "2"], &["plum", "", "4"], &["kiwi", "2", "0.5"]];

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(num(eval1(DATA, "=1+2*3")), 7.0);
        assert_eq!(num(eval1(DATA, "=(1+2)*3")), 9.0);
        assert_eq!(num(eval1(DATA, "=2^3^2")), 64.0); // left-assoc like Excel
        assert_eq!(num(eval1(DATA, "=-2^2")), 4.0); // unary minus binds tighter
        assert_eq!(num(eval1(DATA, "=50%")), 0.5);
        assert_eq!(eval1(DATA, "=\"a\"&1&TRUE"), CellValue::Text("a1TRUE".into()));
        assert_eq!(eval1(DATA, "=1/0"), CellValue::Error("#DIV/0!".into()));
        assert_eq!(eval1(DATA, "=B2>2"), CellValue::Bool(true));
        assert_eq!(eval1(DATA, "=A2=\"APPLE\""), CellValue::Bool(true));
        assert_eq!(eval1(DATA, "=\"x\"+1"), CellValue::Error("#VALUE!".into()));
        assert_eq!(eval1(DATA, "=FOO(1)"), CellValue::Error("#NAME?".into()));
        assert_eq!(eval1(DATA, "=nosuchname"), CellValue::Error("#NAME?".into()));
    }

    #[test]
    fn aggregates() {
        assert_eq!(num(eval1(DATA, "=SUM(B2:B5)")), 10.0);
        assert_eq!(num(eval1(DATA, "=SUM(B:B)")), 10.0);
        assert!((num(eval1(DATA, "=AVERAGE(B2:B5)")) - 10.0 / 3.0).abs() < 1e-9);
        assert_eq!(num(eval1(DATA, "=MIN(C2:C5)")), 0.5);
        assert_eq!(num(eval1(DATA, "=MAX(C2:C5)")), 4.0);
        assert_eq!(num(eval1(DATA, "=COUNT(B2:B5)")), 3.0);
        assert_eq!(num(eval1(DATA, "=COUNTA(A1:A5)")), 5.0);
        assert_eq!(num(eval1(DATA, "=SUMPRODUCT(B2:B5,C2:C5)")), 3.0 * 1.5 + 10.0 + 1.0);
        assert_eq!(num(eval1(DATA, "=SUMIF(B2:B5,\">2\",C2:C5)")), 3.5);
        assert_eq!(num(eval1(DATA, "=COUNTIF(A2:A5,\"p*\")")), 2.0);
        assert_eq!(num(eval1(DATA, "=AVERAGEIF(A2:A5,\"<>pear\",C2:C5)")), 2.0);
        assert_eq!(num(eval1(DATA, "=SUMIFS(C2:C5,A2:A5,\"p*\",C2:C5,\">2\")")), 4.0);
    }

    #[test]
    fn logic_and_errors() {
        assert_eq!(eval1(DATA, "=IF(B2>2,\"big\",\"small\")"), CellValue::Text("big".into()));
        assert_eq!(eval1(DATA, "=IFS(B2>5,\"a\",B2>2,\"b\")"), CellValue::Text("b".into()));
        assert_eq!(eval1(DATA, "=AND(TRUE,B2>1)"), CellValue::Bool(true));
        assert_eq!(eval1(DATA, "=OR(FALSE,B2>10)"), CellValue::Bool(false));
        assert_eq!(eval1(DATA, "=NOT(1)"), CellValue::Bool(false));
        assert_eq!(eval1(DATA, "=IFERROR(1/0,\"none\")"), CellValue::Text("none".into()));
        assert_eq!(eval1(DATA, "=ISBLANK(B4)"), CellValue::Bool(true));
        assert_eq!(eval1(DATA, "=ISNUMBER(B2)"), CellValue::Bool(true));
        assert_eq!(eval1(DATA, "=ISNUMBER(A2)"), CellValue::Bool(false));
        assert_eq!(eval1(DATA, "=Z99"), CellValue::Num(0.0));
    }

    #[test]
    fn math_and_text() {
        assert_eq!(num(eval1(DATA, "=ROUND(2.345,2)")), 2.35);
        assert_eq!(num(eval1(DATA, "=ROUND(-2.5,0)")), -3.0);
        assert_eq!(num(eval1(DATA, "=ROUNDUP(2.301,1)")), 2.4);
        assert_eq!(num(eval1(DATA, "=ROUNDDOWN(2.399,1)")), 2.3);
        assert_eq!(num(eval1(DATA, "=ABS(-4)+SQRT(16)+POWER(2,3)+MOD(-7,3)+INT(-1.5)")), 4.0 + 4.0 + 8.0 + 2.0 - 2.0);
        assert_eq!(eval1(DATA, "=SQRT(-1)"), CellValue::Error("#NUM!".into()));
        assert_eq!(num(eval1(DATA, "=LEN(\"hello\")")), 5.0);
        assert_eq!(eval1(DATA, "=LEFT(\"hello\",2)&RIGHT(\"hello\",2)&MID(\"hello\",2,3)"), CellValue::Text("heloell".into()));
        assert_eq!(eval1(DATA, "=UPPER(\"ab\")&LOWER(\"CD\")"), CellValue::Text("ABcd".into()));
        assert_eq!(eval1(DATA, "=TRIM(\"  a   b \")"), CellValue::Text("a b".into()));
        assert_eq!(eval1(DATA, "=CONCAT(A2:A3,\"!\")"), CellValue::Text("applepear!".into()));
        assert_eq!(eval1(DATA, "=TEXTJOIN(\", \",TRUE,A2:A5)"), CellValue::Text("apple, pear, plum, kiwi".into()));
        assert_eq!(eval1(DATA, "=SUBSTITUTE(\"a-b-c\",\"-\",\"+\")"), CellValue::Text("a+b+c".into()));
        assert_eq!(eval1(DATA, "=SUBSTITUTE(\"a-b-c\",\"-\",\"+\",2)"), CellValue::Text("a-b+c".into()));
        assert_eq!(num(eval1(DATA, "=FIND(\"l\",\"hello\")")), 3.0);
        assert_eq!(eval1(DATA, "=FIND(\"z\",\"hello\")"), CellValue::Error("#VALUE!".into()));
        assert_eq!(num(eval1(DATA, "=VALUE(\"1,234.5\")")), 1234.5);
        assert_eq!(eval1(DATA, "=TEXT(1234.5,\"#,0.00\")"), CellValue::Text("1,234.50".into()));
        assert_eq!(eval1(DATA, "=TEXT(0.25,\"0%\")"), CellValue::Text("25%".into()));
    }

    #[test]
    fn dates() {
        let d = num(eval1(DATA, "=DATE(2024,1,31)"));
        assert_eq!(d, 45322.0);
        assert_eq!(num(eval1(DATA, "=YEAR(DATE(2024,1,31))")), 2024.0);
        assert_eq!(num(eval1(DATA, "=MONTH(DATE(2024,13,1))")), 1.0);
        assert_eq!(num(eval1(DATA, "=DAY(EOMONTH(DATE(2024,1,15),1))")), 29.0);
        assert_eq!(num(eval1(DATA, "=YEAR(\"2023-06-05\")")), 2023.0);
        assert_eq!(eval1(DATA, "=TEXT(DATE(2024,3,5),\"yyyy-mm-dd\")"), CellValue::Text("2024-03-05".into()));
        assert!(num(eval1(DATA, "=TODAY()")) > 45000.0);
        assert!(num(eval1(DATA, "=NOW()")) >= num(eval1(DATA, "=TODAY()")));
    }

    #[test]
    fn lookups() {
        assert_eq!(num(eval1(DATA, "=VLOOKUP(\"pear\",A2:C5,3,FALSE)")), 2.0);
        assert_eq!(eval1(DATA, "=VLOOKUP(\"fig\",A2:C5,3,FALSE)"), CellValue::Error("#N/A".into()));
        assert_eq!(eval1(DATA, "=XLOOKUP(\"plum\",A2:A5,C2:C5)"), CellValue::Num(4.0));
        assert_eq!(eval1(DATA, "=XLOOKUP(\"fig\",A2:A5,C2:C5,\"none\")"), CellValue::Text("none".into()));
        assert_eq!(num(eval1(DATA, "=MATCH(\"kiwi\",A2:A5,0)")), 4.0);
        assert_eq!(num(eval1(DATA, "=INDEX(A2:C5,2,3)")), 2.0);
        assert_eq!(num(eval1(DATA, "=INDEX(C2:C5,MATCH(\"plum\",A2:A5,0))")), 4.0);
        let sorted: &[&[&str]] = &[&["k", "v"], &["1", "a"], &["5", "b"], &["10", "c"]];
        assert_eq!(eval1(sorted, "=VLOOKUP(7,A2:B4,2)"), CellValue::Text("b".into()));
        assert_eq!(num(eval1(sorted, "=MATCH(7,A2:A4)")), 2.0);
    }

    #[test]
    fn grid_formulas_cycles_and_sheets() {
        let s = sheet("Main", &[&["a", "b", "c"], &["1", "=A2*2", "=B2+A2"], &["=C3", "=A3", ""], &["=B4", "=A4", ""]]);
        let g = evaluate_grid_with(&s, &[]);
        assert_eq!(g[1][1], CellValue::Num(2.0));
        assert_eq!(g[1][2], CellValue::Num(3.0));
        assert_eq!(g[2][0], CellValue::Num(0.0)); // C3 is empty → 0
        assert_eq!(g[3][0], CellValue::Error("#CIRC!".into()));
        let other = sheet("Rates", &[&["k", "v"], &["x", "0.2"]]);
        let main = sheet("Main", &[&["v"], &["=Rates!B2*10"], &["='Rates'!B2"], &["=Nope!A1"]]);
        let book = vec![main.clone(), other];
        let g = evaluate_grid_with(&main, &book);
        assert_eq!(g[1][0], CellValue::Num(2.0));
        assert_eq!(g[2][0], CellValue::Num(0.2));
        assert_eq!(g[3][0], CellValue::Error("#REF!".into()));
    }

    #[test]
    fn deep_running_total_does_not_overflow() {
        let mut rows = vec![vec!["n".to_string(), "total".to_string()]];
        for i in 0..5000 {
            rows.push(vec!["1".into(), if i == 0 { "=A2".into() } else { format!("=B{}+A{}", i + 1, i + 2) }]);
        }
        let s = DataSheet { name: "T".into(), cells: rows, ..DataSheet::default() };
        let g = evaluate_grid(&s);
        assert_eq!(g[5000][1], CellValue::Num(5000.0));
    }

    #[test]
    fn shifting_references() {
        assert_eq!(shift_formula("=A1+$B$2+C$3+$D4", 1, 1), "=B2+$B$2+D$3+$D5");
        assert_eq!(shift_formula("=SUM(A1:A3)", 2, 0), "=SUM(A3:A5)");
        assert_eq!(shift_formula("=A1", -1, 0), "=#REF!");
        assert_eq!(shift_formula("=\"A1\"&A1", 1, 0), "=\"A1\"&A2");
        assert_eq!(shift_formula("=Sheet2!A1*LOG10(2)", 1, 0), "=Sheet2!A2*LOG10(2)");
        assert_eq!(shift_formula("=SUM(A:A)", 0, 1), "=SUM(B:B)");
        assert_eq!(shift_formula("plain", 1, 1), "plain");
    }

    #[test]
    fn addresses_and_inputs() {
        assert_eq!(col_name(0), "A");
        assert_eq!(col_name(25), "Z");
        assert_eq!(col_name(26), "AA");
        assert_eq!(col_name(701), "ZZ");
        assert_eq!(col_index("AA"), Some(26));
        assert_eq!(parse_address("$C$10"), Some((9, 2)));
        assert_eq!(parse_input("1,234.5"), Scalar::Num(1234.5));
        assert_eq!(parse_input("$12"), Scalar::Num(12.0));
        assert_eq!(parse_input("15%"), Scalar::Num(0.15));
        assert_eq!(parse_input("TRUE"), Scalar::Bool(true));
        assert_eq!(parse_input("1,2"), Scalar::Text("1,2".into()));
        assert_eq!(parse_input("'007"), Scalar::Text("007".into()));
    }

    #[test]
    fn evaluate_to_table_honours_types() {
        let mut s = sheet("T", &[&["When", "Amount", "Calc", ""], &["2024-01-31", "10", "=B2*2", ""], &["", "", "", ""], &["2024-02-01", "x", "=B4*2", ""]]);
        s.col_types = vec!["Date".into(), "Whole Number".into()];
        let t = evaluate(&s);
        assert_eq!(t.columns, vec!["When", "Amount", "Calc"]);
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.types[0], ColType::Date);
        assert_eq!(t.rows[0][0], Scalar::Num(45322.0));
        assert_eq!(t.rows[0][2], Scalar::Num(20.0));
        assert_eq!(t.rows[1][1], Scalar::Null);
        assert_eq!(t.rows[1][2], Scalar::Null); // #VALUE! → blank in the model
    }
}
