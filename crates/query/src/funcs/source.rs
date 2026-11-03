//! Data sources: files, folders, the current workbook, CSV, JSON, text lines and binaries.

use std::rc::Rc;

use super::{arg, err, text_arg};
use crate::Result;
use crate::m::eval::{Evaluator, check};
use crate::value::{MType, Record, Table, Value};

fn binary(v: &Value) -> Result<Rc<Vec<u8>>> {
    match check(v.clone())? {
        Value::Binary(b) => Ok(b),
        Value::Text(t) => Ok(Rc::new(t.as_bytes().to_vec())),
        other => err(format!("expected a binary, got {}", other.kind())),
    }
}

/// Bytes as text: UTF-8 (with or without BOM), UTF-16 with a BOM, else Windows-1252-ish Latin-1.
pub fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        let le = bytes.starts_with(&[0xFF, 0xFE]);
        let units: Vec<u16> = bytes
            .get(2..)
            .unwrap_or_default()
            .chunks(2)
            .map(|c| {
                let (x, y) = (c.first().copied().unwrap_or(0) as u16, c.get(1).copied().unwrap_or(0) as u16);
                if le { x | (y << 8) } else { (x << 8) | y }
            })
            .collect();
        return String::from_utf16_lossy(&units);
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|b| *b as char).collect(),
    }
}

/// Bytes as text in a code page (`Encoding=`): 65001 UTF-8, 1200 UTF-16LE, 1252 / 28591
/// Windows Latin-1; anything else (or none) is detected.
pub fn decode_with(bytes: &[u8], encoding: Option<f64>) -> String {
    match encoding.map(|e| e as i64) {
        Some(1252) | Some(28591) | Some(1250) | Some(437) | Some(850) if std::str::from_utf8(bytes).is_err() => {
            bytes.iter().map(|b| *b as char).collect()
        }
        Some(1200) if !bytes.starts_with(&[0xFF, 0xFE]) => {
            let units: Vec<u16> =
                bytes.chunks(2).map(|c| c.first().copied().unwrap_or(0) as u16 | ((c.get(1).copied().unwrap_or(0) as u16) << 8)).collect();
            String::from_utf16_lossy(&units)
        }
        _ => decode(bytes),
    }
}

fn hex(b: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push(H[(x >> 4) as usize] as char);
        s.push(H[(x & 15) as usize] as char);
    }
    s
}

/// Percent-encodes a URL query component.
pub fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `Web.Contents(url, [RelativePath, Query, Headers])` as one string for the host: the URL with
/// the path and query applied, then `\u{1}Name: value` per header.
fn web_url(url: &str, opts: &Value) -> String {
    let mut u = url.to_string();
    let mut headers = String::new();
    if let Value::Record(r) = opts {
        if let Some(Value::Text(p)) = r.get("RelativePath") {
            if !u.ends_with('/') && !p.starts_with('/') {
                u.push('/');
            }
            u.push_str(p);
        }
        if let Some(Value::Record(q)) = r.get("Query") {
            for (k, v) in q.names.iter().zip(q.values.iter()) {
                u.push(if u.contains('?') { '&' } else { '?' });
                u.push_str(&format!("{}={}", url_encode(k), url_encode(&v.display())));
            }
        }
        if let Some(Value::Record(h)) = r.get("Headers") {
            for (k, v) in h.names.iter().zip(h.values.iter()) {
                headers.push_str(&format!("\u{1}{k}: {}", v.display()));
            }
        }
    }
    u + &headers
}

/// A table as CSV text (header row, quoted where needed), for scripts.
pub fn table_csv(t: &Table) -> String {
    let q = |s: &str| if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() };
    let mut out = t.columns.iter().map(|c| q(c)).collect::<Vec<_>>().join(",");
    out.push('\n');
    for r in &t.rows {
        out.push_str(&r.iter().map(|v| if v.is_null() { String::new() } else { q(&v.display()) }).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    out
}

/// Parses delimited text into rows of text fields (quotes, doubled quotes, newlines in quotes).
pub fn parse_csv(text: &str, delim: char, quotes: bool) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut in_q = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_q {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_q = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if quotes && field.is_empty() => in_q = true,
            c if c == delim => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// Guesses the delimiter of delimited text from its first lines.
pub fn sniff_delimiter(text: &str) -> char {
    let lines: Vec<&str> = text.lines().take(10).collect();
    let mut best = (',', 0usize);
    for d in [',', ';', '\t', '|'] {
        let counts: Vec<usize> = lines.iter().map(|l| l.matches(d).count()).collect();
        let min = counts.iter().copied().min().unwrap_or(0);
        if min > best.1 {
            best = (d, min);
        }
    }
    best.0
}

pub fn csv_table(text: &str, delim: char, quotes: bool, columns: Option<usize>) -> Table {
    let rows = parse_csv(text, delim, quotes);
    let width = columns.unwrap_or_else(|| rows.iter().map(Vec::len).max().unwrap_or(0));
    let columns: Vec<String> = (1..=width).map(|i| format!("Column{i}")).collect();
    let rows = rows
        .into_iter()
        .map(|r| {
            let mut v: Vec<Value> = r.into_iter().take(width).map(Value::text).collect();
            v.resize(width, Value::Null);
            v
        })
        .collect();
    Table { types: vec![MType::Any; width], columns, rows }
}

/// A JSON value as M: objects are records, arrays lists.
pub fn json_value(j: &serde_json::Value) -> Value {
    match j {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Logical(*b),
        serde_json::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(f64::NAN)),
        serde_json::Value::String(s) => Value::text(s),
        serde_json::Value::Array(a) => Value::list(a.iter().map(json_value).collect()),
        serde_json::Value::Object(o) => Value::record(Record { names: o.keys().cloned().collect(), values: o.values().map(json_value).collect() }),
    }
}

pub fn call(ev: &Evaluator, name: &str, a: &[Value]) -> Option<Result<Value>> {
    let r = match name {
        "Sqlite.Database" | "PostgreSQL.Database" | "MySQL.Database" | "Sql.Database" | "Sql.Databases" | "Odbc.Query" | "Odbc.DataSource"
        | "Parquet.Document" | "Xml.Tables" | "Xml.Document" | "OData.Feed" => (|| {
            let mut texts = Vec::new();
            let mut query = None;
            for v in a {
                match v {
                    Value::Text(t) => texts.push(t.to_string()),
                    Value::Number(n) => texts.push(n.to_string()),
                    // Parquet is binary: hand it over intact (hex), not as text.
                    Value::Binary(b) if name == "Parquet.Document" => texts.push(format!("\u{1}hex:{}", hex(b))),
                    Value::Binary(b) => texts.push(decode(b)),
                    Value::Record(r) => {
                        for (k, v) in r.names.iter().zip(r.values.iter()) {
                            match (k.as_str(), v) {
                                ("Query", Value::Text(q)) => query = Some(q.to_string()),
                                (_, Value::Text(t)) => texts.push(format!("\u{1}opt:{k}={t}")),
                                (_, Value::Number(n)) => texts.push(format!("\u{1}opt:{k}={n}")),
                                (_, Value::Logical(b)) => texts.push(format!("\u{1}opt:{k}={b}")),
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            ev.host.database(name, &texts, query.as_deref()).map(Value::table)
        })(),
        "Web.Page" => (|| {
            let html = match arg(a, 0) {
                Value::Text(t) => t.to_string(),
                v => decode(&binary(v)?),
            };
            Ok(Value::table(super::html::web_page(&html)))
        })(),
        "Python.Execute" => (|| {
            let script = text_arg(a, 0, name)?;
            let mut texts = vec![script];
            if let Value::Record(r) = arg(a, 1) {
                for (k, v) in r.names.iter().zip(r.values.iter()) {
                    if let Value::Table(t) = check(v.clone())? {
                        texts.push(format!("\u{1}opt:{k}={}", table_csv(&t)));
                    }
                }
            }
            ev.host.database(name, &texts, None).map(Value::table)
        })(),
        "Excel.CurrentWorkbook" => ev.host.current_workbook().map(Value::table),
        "File.Contents" => text_arg(a, 0, name).and_then(|p| ev.host.file_contents(&p)).map(|b| Value::Binary(Rc::new(b))),
        "Web.Contents" => (|| {
            let url = web_url(&text_arg(a, 0, name)?, arg(a, 1));
            ev.host.web_contents(&url).map(|b| Value::Binary(Rc::new(b)))
        })(),
        "Folder.Files" | "Folder.Contents" => text_arg(a, 0, name).and_then(|p| ev.host.folder_files(&p)).map(Value::table),
        "Excel.Workbook" => binary(arg(a, 0)).and_then(|b| ev.host.excel_workbook(&b, matches!(arg(a, 1), Value::Logical(true)))).map(Value::table),
        "Csv.Document" => (|| {
            let encoding = match arg(a, 1) {
                Value::Record(r) => r.get("Encoding").and_then(Value::as_number),
                _ => arg(a, 3).as_number(),
            };
            let text = match arg(a, 0) {
                Value::Text(t) => t.to_string(),
                v => decode_with(&binary(v)?, encoding),
            };
            let (mut delim, mut quotes, mut cols) = (None, true, None);
            match arg(a, 1) {
                Value::Record(r) => {
                    delim = r.get("Delimiter").and_then(Value::as_text).and_then(|d| d.chars().next());
                    quotes = r.get("QuoteStyle").and_then(Value::as_number) != Some(0.0);
                    cols = r.get("Columns").and_then(Value::as_number).map(|n| n as usize);
                }
                Value::Number(n) => cols = Some(*n as usize),
                _ => {}
            }
            if let Some(d) = arg(a, 2).as_text().and_then(|d| d.chars().next()) {
                delim = Some(d);
            }
            let delim = delim.unwrap_or_else(|| sniff_delimiter(&text));
            Ok(Value::table(csv_table(&text, delim, quotes, cols.filter(|c| *c <= 16_384))))
        })(),
        "Json.Document" => (|| {
            let text = match arg(a, 0) {
                Value::Text(t) => t.to_string(),
                v => decode(&binary(v)?),
            };
            let j: serde_json::Value = serde_json::from_str(&text)
                .map_err(|e| crate::Error::Eval(format!("DataFormat.Error: We found extra characters at the end of JSON input ({e}).")))?;
            Ok(json_value(&j))
        })(),
        "Lines.FromBinary" => binary(arg(a, 0)).map(|b| Value::list(decode(&b).lines().map(Value::text).collect())),
        "Lines.FromText" => text_arg(a, 0, name).map(|t| Value::list(t.lines().map(Value::text).collect())),
        "Lines.ToText" => super::list_arg(a, 0, name)
            .map(|l| Value::text(l.iter().map(|v| v.as_text().unwrap_or_default().to_string()).collect::<Vec<_>>().join("\r\n"))),
        "Text.FromBinary" => binary(arg(a, 0)).map(|b| Value::text(decode(&b))),
        "Text.ToBinary" | "Binary.FromText" => text_arg(a, 0, name).map(|t| Value::Binary(Rc::new(t.into_bytes()))),
        "Binary.ToText" => binary(arg(a, 0)).map(|b| Value::text(decode(&b))),
        _ => return None,
    };
    Some(r)
}
