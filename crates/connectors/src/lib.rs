//! The desktop host for Power Query: files, folders, Excel workbooks, the web and databases.
//!
//! [`DesktopHost`] implements [`Host`] for the M engine:
//! - `File.Contents`, `Folder.Files` (recursive, with dates) — [`files`];
//! - `Excel.Workbook` (xlsx, xlsm, xlsb, xls, ods via calamine) — [`excel`];
//! - `Web.Contents` (HTTPS through `HTTPS_PROXY`, extra roots from `SSL_CERT_FILE`) and
//!   `OData.Feed` — [`web`];
//! - `Sqlite.Database`, `PostgreSQL.Database`, `MySQL.Database` (navigation tables, or a native
//!   query) and the not-yet connectors (`Sql.Database`, ODBC) — [`db`];
//! - `Xml.Tables` / `Xml.Document` — [`xml`]; `Parquet.Document` — [`parquet_doc`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod db;
pub mod excel;
pub mod files;
pub mod parquet_doc;
pub mod python;
pub mod web;
pub mod xml;

use vizcraft_query::m::eval::Host;
use vizcraft_query::{Error, MType, Record, Result, Table, Value};

/// Reads files, folders, workbooks, the web and databases.
#[derive(Default)]
pub struct DesktopHost;

impl Host for DesktopHost {
    fn file_contents(&self, path: &str) -> Result<Vec<u8>> {
        files::read(path)
    }
    fn folder_files(&self, path: &str) -> Result<Table> {
        files::folder(path)
    }
    fn excel_workbook(&self, bytes: &[u8], use_headers: bool) -> Result<Table> {
        excel::workbook(bytes, use_headers)
    }
    fn web_contents(&self, url: &str) -> Result<Vec<u8>> {
        web::get(url)
    }
    fn database(&self, kind: &str, args: &[String], query: Option<&str>) -> Result<Table> {
        let a = Args::parse(args);
        match kind {
            "Sqlite.Database" => db::sqlite(&a, query),
            "PostgreSQL.Database" => db::postgres(&a, query),
            "MySQL.Database" => db::mysql(&a, query),
            "Sql.Database" | "Sql.Databases" => db::sql_server(&a, query),
            "Odbc.Query" | "Odbc.DataSource" => db::odbc(&a, query),
            "Xml.Tables" => xml::tables(a.pos(0).unwrap_or_default()),
            "Xml.Document" => xml::document(a.pos(0).unwrap_or_default()),
            "Parquet.Document" => parquet_doc::read(&a),
            "OData.Feed" => web::odata(&a),
            "Python.Execute" => python::execute(&a),
            other => Err(Error::Eval(format!("The {other} connector isn't available."))),
        }
    }
}

/// A connector function's arguments: positional texts, binaries (hex) and record options.
#[derive(Clone, Debug, Default)]
pub struct Args {
    pub positional: Vec<String>,
    pub options: Vec<(String, String)>,
    pub binary: Option<Vec<u8>>,
}

impl Args {
    pub fn parse(args: &[String]) -> Args {
        let mut a = Args::default();
        for s in args {
            if let Some(o) = s.strip_prefix("\u{1}opt:") {
                let (k, v) = o.split_once('=').unwrap_or((o, ""));
                a.options.push((k.to_string(), v.to_string()));
            } else if let Some(h) = s.strip_prefix("\u{1}hex:") {
                a.binary = Some(unhex(h));
            } else {
                a.positional.push(s.clone());
            }
        }
        a
    }
    pub fn pos(&self, i: usize) -> Option<&str> {
        self.positional.get(i).map(String::as_str)
    }
    pub fn opt(&self, k: &str) -> Option<&str> {
        self.options.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str())
    }
}

fn unhex(h: &str) -> Vec<u8> {
    let b = h.as_bytes();
    let v = |c: u8| match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    };
    b.chunks(2).map(|p| (v(p.first().copied().unwrap_or(b'0')) << 4) | v(p.get(1).copied().unwrap_or(b'0'))).collect()
}

/// A table from records whose fields may differ: the columns are the union of the field names,
/// in the order first seen.
pub fn table_from_records(recs: &[Record]) -> Table {
    let mut cols: Vec<String> = Vec::new();
    for r in recs {
        for n in &r.names {
            if !cols.contains(n) {
                cols.push(n.clone());
            }
        }
    }
    let rows = recs.iter().map(|r| cols.iter().map(|c| r.get(c).cloned().unwrap_or(Value::Null)).collect()).collect();
    Table::new(cols, rows)
}

/// Sets column types from the values: a column whose non-null values are all numbers becomes
/// a number column, and so on.
pub fn infer_types(t: &mut Table) {
    let mut types = Vec::with_capacity(t.columns.len());
    for c in 0..t.columns.len() {
        let mut ty: Option<MType> = None;
        let mut mixed = false;
        for r in t.rows.iter().take(1000) {
            let k = match r.get(c) {
                Some(Value::Number(_)) => MType::Number,
                Some(Value::Text(_)) => MType::Text,
                Some(Value::Logical(_)) => MType::Logical,
                Some(Value::Date(_)) => MType::Date,
                Some(Value::DateTime(_)) => MType::DateTime,
                Some(Value::Time(_)) => MType::Time,
                Some(Value::Table(_)) => MType::Table,
                Some(Value::Record(_)) => MType::Record,
                Some(Value::List(_)) => MType::List,
                Some(Value::Binary(_)) => MType::Binary,
                _ => continue,
            };
            match ty {
                None => ty = Some(k),
                Some(x) if x != k => mixed = true,
                _ => {}
            }
        }
        types.push(if mixed { MType::Any } else { ty.unwrap_or(MType::Any) });
    }
    t.types = types;
}

/// Promotes the first row to column names (`Table.PromoteHeaders`).
pub fn promote_headers(t: Table) -> Table {
    let Some(first) = t.rows.first() else { return t };
    let mut names: Vec<String> = Vec::new();
    for (i, v) in first.iter().enumerate() {
        let mut n = match v {
            Value::Null => format!("Column{}", i + 1),
            v => v.display(),
        };
        if n.trim().is_empty() {
            n = format!("Column{}", i + 1);
        }
        let base = n.clone();
        let mut k = 1;
        while names.contains(&n) {
            n = format!("{base}_{k}");
            k += 1;
        }
        names.push(n);
    }
    let rows = t.rows.into_iter().skip(1).collect();
    let mut out = Table::new(names, rows);
    infer_types(&mut out);
    out
}

pub(crate) fn err<T>(m: impl Into<String>) -> Result<T> {
    Err(Error::Eval(m.into()))
}

#[cfg(test)]
mod tests;
