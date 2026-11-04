//! VizCraft SQL: an embedded analytical SQL engine (DuckDB) inside the BI studio.
//!
//! * [`Engine`] wraps an in-process DuckDB database. Every table loaded into the semantic model
//!   is registered as a SQL table (typed: dates become `DATE`, date/times `TIMESTAMP`, …); files
//!   (CSV, Parquet, JSON) are attached as views read by DuckDB's own readers; SQLite / PostgreSQL /
//!   MySQL databases can be attached through DuckDB extensions when they're available.
//! * A per-project **warehouse** (`<project>.duckdb` next to the `.vizc`) keeps the tables you
//!   create with `CREATE TABLE … AS …`; it's the default schema for new objects.
//! * [`worker::Worker`] runs scripts on a background thread (cancellable).
//! * [`python`] runs Python (pandas) over a query result.
//! * [`text`] splits scripts into statements and tokenizes SQL for highlighting.
//!
//! The DuckDB backend is the `duckdb` cargo feature (on by default). Without it the engine
//! reports that SQL isn't available instead of failing to build.

pub mod python;
pub mod text;
pub mod worker;

#[cfg(feature = "duckdb")]
mod duck;
#[cfg(feature = "duckdb")]
pub use duck::Engine;

#[cfg(not(feature = "duckdb"))]
mod stub;
#[cfg(not(feature = "duckdb"))]
pub use stub::Engine;

use vizcraft_core::build::{ColType, TableData};
use vizcraft_core::scalar::Scalar;

/// Days between the spreadsheet epoch (1899-12-30, serial 0) and the Unix epoch.
pub const UNIX_EPOCH_SERIAL: f64 = 25569.0;
const MICROS_PER_DAY: f64 = 86_400_000_000.0;

/// True when this build has a SQL engine.
pub const AVAILABLE: bool = cfg!(feature = "duckdb");

pub type Result<T> = std::result::Result<T, String>;

/// One result set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueryResult {
    pub columns: Vec<String>,
    /// The engine's type names (`VARCHAR`, `BIGINT`, `DATE`, …).
    pub types: Vec<String>,
    /// The same types as model column types (dates as serial numbers in `rows`).
    pub col_types: Vec<ColType>,
    /// Rows (capped at the display limit; see `truncated`).
    pub rows: Vec<Vec<Scalar>>,
    /// More rows exist than were fetched.
    pub truncated: bool,
    pub elapsed_ms: f64,
    /// Rows inserted / updated / deleted, for DML.
    pub rows_affected: Option<u64>,
    /// The statement that produced it.
    pub sql: String,
}

impl QueryResult {
    /// As model table data (for "Load to model" and the Python bridge).
    pub fn to_table_data(&self, name: &str) -> TableData {
        TableData { name: name.to_string(), columns: self.columns.clone(), types: self.col_types.clone(), rows: self.rows.clone() }
    }

    /// As a Power Query table, with dates as M dates.
    pub fn to_m(&self) -> vizcraft_query::Table {
        use vizcraft_query::{MType, Value};
        let types: Vec<MType> = self
            .col_types
            .iter()
            .map(|t| match t {
                ColType::Text => MType::Text,
                ColType::Number => MType::Number,
                ColType::Integer => MType::Int64,
                ColType::Date => MType::Date,
                ColType::DateTime => MType::DateTime,
                ColType::Time => MType::Time,
                ColType::Bool => MType::Logical,
                ColType::Any => MType::Any,
            })
            .collect();
        let rows = self
            .rows
            .iter()
            .map(|r| {
                r.iter()
                    .enumerate()
                    .map(|(i, v)| match (self.col_types.get(i).copied().unwrap_or_default(), v) {
                        (_, Scalar::Null) => Value::Null,
                        (ColType::Date, Scalar::Num(n)) => Value::Date(n.floor() as i64),
                        (ColType::DateTime, Scalar::Num(n)) => Value::DateTime(*n),
                        (ColType::Time, Scalar::Num(n)) => Value::Time(*n),
                        (_, Scalar::Num(n)) => Value::Number(*n),
                        (_, Scalar::Bool(b)) => Value::Logical(*b),
                        (_, Scalar::Text(t)) => Value::text(t),
                    })
                    .collect()
            })
            .collect();
        let mut t = vizcraft_query::Table::new(self.columns.clone(), rows);
        t.types = types;
        t
    }

    /// Display text for a cell (dates formatted by column type).
    pub fn cell_text(&self, r: usize, c: usize) -> String {
        let v = self.rows.get(r).and_then(|row| row.get(c)).unwrap_or(&Scalar::Null);
        display(v, self.col_types.get(c).copied().unwrap_or_default())
    }

    /// Tab-separated text (with a header row) for the clipboard.
    pub fn to_tsv(&self, rows: Option<std::ops::Range<usize>>, cols: Option<std::ops::Range<usize>>) -> String {
        let rows = rows.unwrap_or(0..self.rows.len());
        let cols = cols.unwrap_or(0..self.columns.len());
        let mut out = String::new();
        out.push_str(&cols.clone().filter_map(|c| self.columns.get(c).cloned()).collect::<Vec<_>>().join("\t"));
        out.push('\n');
        for r in rows {
            if r >= self.rows.len() {
                break;
            }
            let line: Vec<String> = cols.clone().map(|c| self.cell_text(r, c).replace(['\t', '\n'], " ")).collect();
            out.push_str(&line.join("\t"));
            out.push('\n');
        }
        out
    }

    /// CSV text with a header (dates ISO), as the Python bridge reads it.
    pub fn to_csv(&self) -> String {
        let mut out = String::new();
        let q = |s: &str| {
            if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
        };
        out.push_str(&self.columns.iter().map(|c| q(c)).collect::<Vec<_>>().join(","));
        out.push('\n');
        for r in 0..self.rows.len() {
            let line: Vec<String> = (0..self.columns.len())
                .map(|c| {
                    let v = self.rows.get(r).and_then(|row| row.get(c)).unwrap_or(&Scalar::Null);
                    match v {
                        Scalar::Null => String::new(),
                        Scalar::Bool(b) => if *b { "True".into() } else { "False".into() },
                        _ => q(&self.cell_text(r, c)),
                    }
                })
                .collect();
            out.push_str(&line.join(","));
            out.push('\n');
        }
        out
    }

    /// Sorts rows by a column.
    pub fn sort_by(&mut self, col: usize, descending: bool) {
        self.rows.sort_by(|a, b| {
            let o = vizcraft_core::scalar::compare(a.get(col).unwrap_or(&Scalar::Null), b.get(col).unwrap_or(&Scalar::Null));
            if descending { o.reverse() } else { o }
        });
    }
}

/// Display text for a value of a column type.
pub fn display(v: &Scalar, ty: ColType) -> String {
    match (ty, v) {
        (_, Scalar::Null) => "NULL".into(),
        (ColType::Date, Scalar::Num(n)) => serial_to_date(*n).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_else(|| n.to_string()),
        (ColType::DateTime, Scalar::Num(n)) => serial_to_datetime(*n).map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_else(|| n.to_string()),
        (ColType::Time, Scalar::Num(n)) => {
            let secs = (n.fract().abs() * 86400.0).round() as i64;
            format!("{:02}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60)
        }
        (ColType::Integer, Scalar::Num(n)) if n.fract() == 0.0 && n.abs() < 1e15 => format!("{}", *n as i64),
        (_, Scalar::Num(n)) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{}", *n as i64)
            } else {
                let s = format!("{n:.6}");
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
        }
        (_, Scalar::Bool(b)) => if *b { "true".into() } else { "false".into() },
        (_, Scalar::Text(t)) => t.clone(),
    }
}

/// A date serial (days since 1899-12-30) as a calendar date.
pub fn serial_to_date(serial: f64) -> Option<chrono::NaiveDate> {
    let base = chrono::NaiveDate::from_ymd_opt(1899, 12, 30)?;
    base.checked_add_signed(chrono::Duration::days(serial.floor() as i64))
}

/// A date/time serial as a timestamp.
pub fn serial_to_datetime(serial: f64) -> Option<chrono::NaiveDateTime> {
    let d = serial_to_date(serial)?;
    let micros = ((serial - serial.floor()) * MICROS_PER_DAY).round() as i64;
    d.and_hms_opt(0, 0, 0)?.checked_add_signed(chrono::Duration::microseconds(micros))
}

/// Days since the Unix epoch (DuckDB `DATE`) for a serial.
pub fn serial_to_unix_days(serial: f64) -> i32 {
    (serial.floor() - UNIX_EPOCH_SERIAL) as i32
}
/// A serial for days since the Unix epoch.
pub fn unix_days_to_serial(days: i64) -> f64 {
    days as f64 + UNIX_EPOCH_SERIAL
}
/// Microseconds since the Unix epoch (DuckDB `TIMESTAMP`) for a serial.
pub fn serial_to_unix_micros(serial: f64) -> i64 {
    ((serial - UNIX_EPOCH_SERIAL) * MICROS_PER_DAY).round() as i64
}
/// A serial for microseconds since the Unix epoch.
pub fn unix_micros_to_serial(micros: i64) -> f64 {
    micros as f64 / MICROS_PER_DAY + UNIX_EPOCH_SERIAL
}

/// The DuckDB type a model column type is registered as.
pub fn sql_type(ty: ColType) -> &'static str {
    match ty {
        ColType::Text | ColType::Any => "VARCHAR",
        ColType::Number => "DOUBLE",
        ColType::Integer => "BIGINT",
        ColType::Date => "DATE",
        ColType::DateTime => "TIMESTAMP",
        ColType::Time => "TIME",
        ColType::Bool => "BOOLEAN",
    }
}

/// The model column type for a DuckDB type name.
pub fn col_type_of(sql: &str) -> ColType {
    let s = sql.trim().to_ascii_uppercase();
    let base = s.split(['(', ' ']).next().unwrap_or("");
    match base {
        "VARCHAR" | "TEXT" | "STRING" | "CHAR" | "BPCHAR" | "UUID" | "ENUM" => ColType::Text,
        "TINYINT" | "SMALLINT" | "INTEGER" | "INT" | "BIGINT" | "HUGEINT" | "UTINYINT" | "USMALLINT" | "UINTEGER" | "UBIGINT" | "UHUGEINT" | "INT8" | "INT4" | "INT2" => {
            ColType::Integer
        }
        "DOUBLE" | "FLOAT" | "REAL" | "DECIMAL" | "NUMERIC" | "FLOAT8" | "FLOAT4" => ColType::Number,
        "DATE" => ColType::Date,
        "TIMESTAMP" | "DATETIME" | "TIMESTAMPTZ" | "TIMESTAMP_S" | "TIMESTAMP_MS" | "TIMESTAMP_NS" => ColType::DateTime,
        "TIME" => ColType::Time,
        "BOOLEAN" | "BOOL" => ColType::Bool,
        _ if s.starts_with("TIMESTAMP") => ColType::DateTime,
        _ => ColType::Any,
    }
}

/// Quotes an identifier for SQL.
pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Quotes a string literal for SQL.
pub fn quote_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// What a catalog object is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ObjectKind {
    /// A semantic-model table (registered from the last refresh).
    Model,
    /// A table in the project warehouse (`<project>.duckdb`).
    Warehouse,
    /// A view (warehouse or session).
    View,
    /// A file attached as a view (CSV, Parquet, JSON).
    File,
    /// A table of an attached external database.
    External,
}

impl ObjectKind {
    pub fn label(self) -> &'static str {
        match self {
            ObjectKind::Model => "Model tables",
            ObjectKind::Warehouse => "Warehouse",
            ObjectKind::View => "Views",
            ObjectKind::File => "Files",
            ObjectKind::External => "Attached databases",
        }
    }
}

/// A table or view the engine knows.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogObject {
    pub kind: ObjectKind,
    pub database: String,
    pub schema: String,
    pub name: String,
    pub columns: Vec<(String, String)>,
    /// For files: the source path.
    pub source: Option<String>,
}

impl CatalogObject {
    /// How to refer to it in SQL (unqualified when it's on the search path).
    pub fn sql_name(&self) -> String {
        match self.kind {
            ObjectKind::Model | ObjectKind::File => quote_ident(&self.name),
            ObjectKind::Warehouse | ObjectKind::View if self.database == "warehouse" && self.schema == "main" => quote_ident(&self.name),
            _ => format!("{}.{}.{}", quote_ident(&self.database), quote_ident(&self.schema), quote_ident(&self.name)),
        }
    }
}

/// The output of one statement of a script.
#[derive(Clone, Debug)]
pub enum StatementOutput {
    Rows(QueryResult),
    /// A plan (EXPLAIN / EXPLAIN ANALYZE): text.
    Plan { sql: String, text: String, elapsed_ms: f64 },
    Error { sql: String, message: String },
}

/// Turns raw engine errors into something readable.
pub fn friendly_error(e: &str) -> String {
    let e = e.trim();
    let e = e.strip_prefix("Error: ").unwrap_or(e);
    if e.contains("Catalog Error") && e.contains("does not exist") {
        return format!("{e}\nTip: model tables appear after a refresh; files can be attached from the object explorer.");
    }
    if e.contains("INTERRUPT") || e.contains("Interrupted") {
        return "The query was cancelled.".into();
    }
    e.to_string()
}

#[cfg(test)]
mod tests;
