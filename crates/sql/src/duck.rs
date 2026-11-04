//! The DuckDB backend.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use duckdb::arrow::datatypes::DataType;
use duckdb::types::{TimeUnit, Value, ValueRef};
use duckdb::{Connection, InterruptHandle};
use vizardcraft_core::build::{ColType, TableData};
use vizardcraft_core::scalar::Scalar;

use crate::{CatalogObject, ObjectKind, QueryResult, Result, StatementOutput, quote_ident, quote_str, text};

/// An in-process DuckDB database: model tables in `memory.main`, attached files in
/// `memory.files`, the project warehouse attached as `warehouse` (the default for new objects).
pub struct Engine {
    conn: Connection,
    warehouse: Option<PathBuf>,
    /// Registered model tables → a fingerprint of their data (to skip unchanged ones).
    registered: BTreeMap<String, u64>,
    /// Attached files: view name → path.
    files: BTreeMap<String, String>,
    /// Attached external databases: alias → description.
    pub attached: BTreeMap<String, String>,
}

fn err(e: duckdb::Error) -> String {
    crate::friendly_error(&e.to_string())
}

impl Engine {
    /// Opens an engine. `warehouse`: the project's `.duckdb` file (created on first use); `None`
    /// keeps warehouse tables in memory (an unsaved project).
    pub fn open(warehouse: Option<&Path>) -> Result<Engine> {
        let conn = Connection::open_in_memory().map_err(err)?;
        let target = match warehouse {
            Some(p) => quote_str(&p.to_string_lossy()),
            None => "':memory:'".to_string(),
        };
        conn.execute_batch(&format!("ATTACH {target} AS warehouse; CREATE SCHEMA IF NOT EXISTS memory.files;")).map_err(err)?;
        let e = Engine { conn, warehouse: warehouse.map(Path::to_path_buf), registered: BTreeMap::new(), files: BTreeMap::new(), attached: BTreeMap::new() };
        e.setup(&e.conn)?;
        Ok(e)
    }

    /// Per-connection settings: new objects go to the warehouse; names resolve warehouse first,
    /// then model tables, then files.
    fn setup(&self, c: &Connection) -> Result<()> {
        c.execute_batch("USE warehouse; SET search_path = 'warehouse.main,memory.main,memory.files';").map_err(err)
    }

    /// Another connection to the same database (for a worker thread).
    pub fn connection(&self) -> Result<Connection> {
        let c = self.conn.try_clone().map_err(err)?;
        self.setup(&c)?;
        Ok(c)
    }

    pub fn warehouse_path(&self) -> Option<&Path> {
        self.warehouse.as_deref()
    }

    pub fn version(&self) -> String {
        self.conn.version().unwrap_or_default()
    }

    pub fn interrupt_handle(&self) -> Arc<InterruptHandle> {
        self.conn.interrupt_handle()
    }

    // ------------------------------------------------------------ model tables

    /// Registers (or replaces) a model table. Skips tables whose data hasn't changed.
    pub fn register_table(&mut self, t: &TableData) -> Result<()> {
        let fp = fingerprint(t);
        if self.registered.get(&t.name) == Some(&fp) {
            return Ok(());
        }
        register(&self.conn, "memory", "main", t)?;
        self.registered.insert(t.name.clone(), fp);
        Ok(())
    }

    /// Registers every model table and drops ones no longer in the model.
    pub fn sync_model<'a>(&mut self, tables: impl IntoIterator<Item = &'a TableData>) -> Vec<(String, String)> {
        let mut errors = Vec::new();
        let mut seen = Vec::new();
        for t in tables {
            seen.push(t.name.clone());
            if let Err(e) = self.register_table(t) {
                errors.push((t.name.clone(), e));
            }
        }
        let stale: Vec<String> = self.registered.keys().filter(|k| !seen.contains(k)).cloned().collect();
        for k in stale {
            let _ = self.conn.execute_batch(&format!("DROP TABLE IF EXISTS memory.main.{}", quote_ident(&k)));
            self.registered.remove(&k);
        }
        errors
    }

    pub fn is_registered(&self, name: &str) -> bool {
        self.registered.keys().any(|k| k.eq_ignore_ascii_case(name))
    }

    pub fn model_tables(&self) -> Vec<String> {
        self.registered.keys().cloned().collect()
    }

    /// Stores table data as a warehouse table (`CREATE OR REPLACE`).
    pub fn save_table(&self, name: &str, t: &TableData) -> Result<()> {
        let mut t = t.clone();
        t.name = name.to_string();
        register(&self.conn, "warehouse", "main", &t)
    }

    // ------------------------------------------------------------ files and databases

    /// Attaches a file as a view read by DuckDB's readers (CSV/TSV, Parquet, JSON; Excel when
    /// the excel extension is available). Returns the view name.
    pub fn attach_file(&mut self, path: &str, name: Option<&str>) -> Result<String> {
        let reader = file_reader(path)?;
        let stem = Path::new(path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
        let name = name.map(str::to_string).unwrap_or_else(|| sanitize(&stem));
        if reader.starts_with("read_xlsx") {
            self.load_extension("excel")?;
        }
        self.conn.execute_batch(&format!("CREATE OR REPLACE VIEW memory.files.{} AS SELECT * FROM {reader}", quote_ident(&name))).map_err(err)?;
        self.files.insert(name.clone(), path.to_string());
        Ok(name)
    }

    pub fn detach_file(&mut self, name: &str) -> Result<()> {
        self.conn.execute_batch(&format!("DROP VIEW IF EXISTS memory.files.{}", quote_ident(name))).map_err(err)?;
        self.files.remove(name);
        Ok(())
    }

    pub fn files(&self) -> &BTreeMap<String, String> {
        &self.files
    }

    /// Loads an extension (`sqlite`, `postgres`, `mysql`, `excel`, …): tries LOAD, then INSTALL
    /// (which needs network access the first time).
    pub fn load_extension(&self, name: &str) -> Result<()> {
        let n = sanitize(name);
        if self.conn.execute_batch(&format!("LOAD {n}")).is_ok() {
            return Ok(());
        }
        self.conn.execute_batch(&format!("INSTALL {n}; LOAD {n};")).map_err(|e| {
            format!(
                "The DuckDB `{n}` extension isn't available offline ({}). Connect to the internet once so DuckDB can install it, or copy the extension into ~/.duckdb/extensions.",
                crate::friendly_error(&e.to_string()).lines().next().unwrap_or_default()
            )
        })
    }

    /// Attaches an external database through a DuckDB extension. `kind`: `sqlite`, `postgres`,
    /// `mysql` or `duckdb`; `target`: a path or a connection string.
    pub fn attach_database(&mut self, kind: &str, target: &str, alias: &str) -> Result<()> {
        let alias = sanitize(alias);
        let kind_l = kind.to_ascii_lowercase();
        let ty = match kind_l.as_str() {
            "sqlite" | "sqlite3" => {
                self.load_extension("sqlite")?;
                "SQLITE"
            }
            "postgres" | "postgresql" => {
                self.load_extension("postgres")?;
                "POSTGRES"
            }
            "mysql" | "mariadb" => {
                self.load_extension("mysql")?;
                "MYSQL"
            }
            "duckdb" => "",
            other => return Err(format!("Unknown database kind `{other}`: use sqlite, postgres, mysql or duckdb.")),
        };
        let ty = if ty.is_empty() { String::new() } else { format!(" (TYPE {ty}, READ_ONLY)") };
        self.conn.execute_batch(&format!("ATTACH {} AS {}{ty}", quote_str(target), quote_ident(&alias))).map_err(err)?;
        self.attached.insert(alias, format!("{kind}: {target}"));
        Ok(())
    }

    pub fn detach_database(&mut self, alias: &str) -> Result<()> {
        self.conn.execute_batch(&format!("DETACH {}", quote_ident(alias))).map_err(err)?;
        self.attached.remove(alias);
        Ok(())
    }

    // ------------------------------------------------------------ queries

    /// Runs one statement and returns its first `cap` rows.
    pub fn query(&self, sql: &str, cap: usize) -> Result<QueryResult> {
        run_query(&self.conn, sql, cap)
    }

    /// Runs a script: every statement, in order (stops at the first error).
    pub fn run_script(&self, sql: &str, cap: usize) -> Vec<StatementOutput> {
        run_script(&self.conn, sql, cap)
    }

    /// The plan of a statement (`EXPLAIN` / `EXPLAIN ANALYZE`).
    pub fn explain(&self, sql: &str, analyze: bool) -> Result<String> {
        explain(&self.conn, sql, analyze)
    }

    /// Exports a query's full result with `COPY … TO` (format from the extension: csv,
    /// parquet, json).
    pub fn export(&self, sql: &str, path: &str) -> Result<()> {
        export(&self.conn, sql, path)
    }

    /// Every table and view, with columns.
    pub fn catalog(&self) -> Result<Vec<CatalogObject>> {
        let mut objs: Vec<CatalogObject> = Vec::new();
        let mut idx: BTreeMap<(String, String, String), usize> = BTreeMap::new();
        let mut st = self
            .conn
            .prepare(
                "SELECT database_name, schema_name, table_name, 'table' FROM duckdb_tables() WHERE NOT internal AND NOT temporary \
                 UNION ALL SELECT database_name, schema_name, view_name, 'view' FROM duckdb_views() WHERE NOT internal AND NOT temporary \
                 ORDER BY 1, 2, 3",
            )
            .map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            let db: String = r.get(0).unwrap_or_default();
            let schema: String = r.get(1).unwrap_or_default();
            let name: String = r.get(2).unwrap_or_default();
            let what: String = r.get(3).unwrap_or_default();
            if db == "system" || db == "temp" {
                continue;
            }
            let kind = if db == "memory" && schema == "files" {
                ObjectKind::File
            } else if db == "memory" && schema == "main" {
                ObjectKind::Model
            } else if db == "warehouse" {
                if what == "view" { ObjectKind::View } else { ObjectKind::Warehouse }
            } else if what == "view" {
                ObjectKind::View
            } else {
                ObjectKind::External
            };
            let source = if kind == ObjectKind::File { self.files.get(&name).cloned() } else { None };
            idx.insert((db.clone(), schema.clone(), name.clone()), objs.len());
            objs.push(CatalogObject { kind, database: db, schema, name, columns: vec![], source });
        }
        drop(rows);
        let mut st = self
            .conn
            .prepare("SELECT database_name, schema_name, table_name, column_name, data_type FROM duckdb_columns() WHERE NOT internal ORDER BY database_name, schema_name, table_name, column_index")
            .map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            let key: (String, String, String) = (r.get(0).unwrap_or_default(), r.get(1).unwrap_or_default(), r.get(2).unwrap_or_default());
            if let Some(i) = idx.get(&key).copied()
                && let Some(o) = objs.get_mut(i)
            {
                o.columns.push((r.get(3).unwrap_or_default(), r.get(4).unwrap_or_default()));
            }
        }
        Ok(objs)
    }

    /// Warehouse tables (for "Load to model").
    pub fn warehouse_tables(&self) -> Vec<String> {
        self.catalog().unwrap_or_default().into_iter().filter(|o| o.kind == ObjectKind::Warehouse).map(|o| o.name).collect()
    }
}

/// A cheap fingerprint of a table's data.
fn fingerprint(t: &TableData) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.columns.hash(&mut h);
    format!("{:?}", t.types).hash(&mut h);
    t.rows.len().hash(&mut h);
    for r in &t.rows {
        for v in r {
            match v {
                Scalar::Null => 0u8.hash(&mut h),
                Scalar::Bool(b) => b.hash(&mut h),
                Scalar::Num(n) => n.to_bits().hash(&mut h),
                Scalar::Text(s) => s.hash(&mut h),
            }
        }
    }
    h.finish()
}

/// A name usable as an identifier.
pub fn sanitize(s: &str) -> String {
    let mut out: String = s.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if out.is_empty() || out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// The DuckDB table function that reads a file.
pub fn file_reader(path: &str) -> Result<String> {
    let lower = path.to_ascii_lowercase();
    let p = quote_str(path);
    Ok(if lower.ends_with(".parquet") || lower.ends_with(".pq") {
        format!("read_parquet({p})")
    } else if lower.ends_with(".json") || lower.ends_with(".jsonl") || lower.ends_with(".ndjson") {
        format!("read_json_auto({p})")
    } else if lower.ends_with(".csv") || lower.ends_with(".tsv") || lower.ends_with(".txt") || lower.ends_with(".csv.gz") {
        format!("read_csv_auto({p})")
    } else if lower.ends_with(".xlsx") {
        format!("read_xlsx({p})")
    } else {
        return Err(format!("VizardCraft SQL can read CSV/TSV, Parquet, JSON and .xlsx files; `{path}` isn't one of those."));
    })
}

/// Creates `catalog.schema."name"` from table data with the Appender.
pub fn register(conn: &Connection, catalog: &str, schema: &str, t: &TableData) -> Result<()> {
    let mut names: Vec<String> = Vec::new();
    for (i, c) in t.columns.iter().enumerate() {
        let mut n = if c.trim().is_empty() { format!("Column{}", i + 1) } else { c.clone() };
        let base = n.clone();
        let mut k = 2;
        while names.iter().any(|x| x.eq_ignore_ascii_case(&n)) {
            n = format!("{base}_{k}");
            k += 1;
        }
        names.push(n);
    }
    let types: Vec<ColType> = (0..names.len()).map(|i| t.types.get(i).copied().unwrap_or_default()).collect();
    let cols: Vec<String> = names.iter().zip(&types).map(|(n, ty)| format!("{} {}", quote_ident(n), crate::sql_type(*ty))).collect();
    let full = format!("{}.{}.{}", quote_ident(catalog), quote_ident(schema), quote_ident(&t.name));
    conn.execute_batch(&format!("CREATE OR REPLACE TABLE {full} ({})", cols.join(", "))).map_err(err)?;
    let mut app = conn.appender_to_catalog_and_db(&t.name, catalog, schema).map_err(err)?;
    for r in &t.rows {
        let vals: Vec<Value> = types.iter().enumerate().map(|(i, ty)| to_value(r.get(i).unwrap_or(&Scalar::Null), *ty)).collect();
        app.append_row(duckdb::appender_params_from_iter(vals)).map_err(err)?;
    }
    app.flush().map_err(err)?;
    Ok(())
}

/// A model value as a DuckDB value of the column's type.
pub fn to_value(v: &Scalar, ty: ColType) -> Value {
    match (ty, v) {
        (_, Scalar::Null) => Value::Null,
        (ColType::Text | ColType::Any, v) => Value::Text(match v {
            Scalar::Text(t) => t.clone(),
            Scalar::Num(n) => crate::display(&Scalar::Num(*n), ColType::Number),
            Scalar::Bool(b) => b.to_string(),
            Scalar::Null => String::new(),
        }),
        (ColType::Number, v) => v.as_f64().map(Value::Double).unwrap_or(Value::Null),
        (ColType::Integer, v) => v.as_f64().filter(|n| n.is_finite()).map(|n| Value::BigInt(n.round() as i64)).unwrap_or(Value::Null),
        (ColType::Date, Scalar::Num(n)) if n.is_finite() => Value::Date32(crate::serial_to_unix_days(*n)),
        (ColType::DateTime, Scalar::Num(n)) if n.is_finite() => Value::Timestamp(TimeUnit::Microsecond, crate::serial_to_unix_micros(*n)),
        (ColType::Time, Scalar::Num(n)) if n.is_finite() => Value::Time64(TimeUnit::Microsecond, (n.fract().abs() * 86_400_000_000.0).round() as i64),
        (ColType::Date | ColType::DateTime, Scalar::Text(s)) => match vizardcraft_core::build::parse_date(s) {
            Some(n) if ty == ColType::Date => Value::Date32(crate::serial_to_unix_days(n)),
            Some(n) => Value::Timestamp(TimeUnit::Microsecond, crate::serial_to_unix_micros(n)),
            None => Value::Null,
        },
        (ColType::Bool, Scalar::Bool(b)) => Value::Boolean(*b),
        (ColType::Bool, Scalar::Num(n)) => Value::Boolean(*n != 0.0),
        (ColType::Bool, Scalar::Text(t)) => match t.to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" => Value::Boolean(true),
            "false" | "no" | "0" => Value::Boolean(false),
            _ => Value::Null,
        },
        _ => Value::Null,
    }
}

/// A DuckDB value as a model value.
pub fn from_value(v: ValueRef<'_>) -> Scalar {
    match v {
        ValueRef::Null => Scalar::Null,
        ValueRef::Boolean(b) => Scalar::Bool(b),
        ValueRef::TinyInt(n) => Scalar::Num(n as f64),
        ValueRef::SmallInt(n) => Scalar::Num(n as f64),
        ValueRef::Int(n) => Scalar::Num(n as f64),
        ValueRef::BigInt(n) => Scalar::Num(n as f64),
        ValueRef::HugeInt(n) => Scalar::Num(n as f64),
        ValueRef::UHugeInt(n) => Scalar::Num(n as f64),
        ValueRef::UTinyInt(n) => Scalar::Num(n as f64),
        ValueRef::USmallInt(n) => Scalar::Num(n as f64),
        ValueRef::UInt(n) => Scalar::Num(n as f64),
        ValueRef::UBigInt(n) => Scalar::Num(n as f64),
        ValueRef::Float(n) => Scalar::Num(n as f64),
        ValueRef::Double(n) => Scalar::Num(n),
        ValueRef::Decimal(d) => d.to_string().parse().map(Scalar::Num).unwrap_or(Scalar::Null),
        ValueRef::Timestamp(u, n) => Scalar::Num(crate::unix_micros_to_serial(u.to_micros(n))),
        ValueRef::Date32(d) => Scalar::Num(crate::unix_days_to_serial(d as i64)),
        ValueRef::Time64(u, n) => Scalar::Num(u.to_micros(n) as f64 / 86_400_000_000.0),
        ValueRef::Text(b) => Scalar::Text(String::from_utf8_lossy(b).into_owned()),
        ValueRef::Blob(b) => Scalar::Text(format!("<blob {} bytes>", b.len())),
        ValueRef::Interval { months, days, nanos } => Scalar::Text(format!("{months} months {days} days {}s", nanos as f64 / 1e9)),
        other => Scalar::Text(value_text(&other.to_owned())),
    }
}

fn value_text(v: &Value) -> String {
    match v {
        Value::Null => "NULL".into(),
        Value::Text(t) | Value::Enum(t) => t.clone(),
        Value::List(l) | Value::Array(l) => format!("[{}]", l.iter().map(value_text).collect::<Vec<_>>().join(", ")),
        Value::Struct(m) => format!("{{{}}}", m.iter().map(|(k, v)| format!("{k}: {}", value_text(v))).collect::<Vec<_>>().join(", ")),
        Value::Map(m) => format!("{{{}}}", m.iter().map(|(k, v)| format!("{}: {}", value_text(k), value_text(v))).collect::<Vec<_>>().join(", ")),
        Value::Union(b) => value_text(b),
        other => match from_value(ValueRef::from(other)) {
            Scalar::Text(t) => t,
            s => s.text(),
        },
    }
}

/// DuckDB type name and model type for an Arrow result type.
fn type_of(dt: &DataType) -> (String, ColType) {
    let (n, t) = match dt {
        DataType::Boolean => ("BOOLEAN", ColType::Bool),
        DataType::Int8 => ("TINYINT", ColType::Integer),
        DataType::Int16 => ("SMALLINT", ColType::Integer),
        DataType::Int32 => ("INTEGER", ColType::Integer),
        DataType::Int64 => ("BIGINT", ColType::Integer),
        DataType::UInt8 => ("UTINYINT", ColType::Integer),
        DataType::UInt16 => ("USMALLINT", ColType::Integer),
        DataType::UInt32 => ("UINTEGER", ColType::Integer),
        DataType::UInt64 => ("UBIGINT", ColType::Integer),
        DataType::Float16 | DataType::Float32 => ("FLOAT", ColType::Number),
        DataType::Float64 => ("DOUBLE", ColType::Number),
        DataType::Decimal128(p, s) | DataType::Decimal256(p, s) => return (format!("DECIMAL({p},{s})"), ColType::Number),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => ("VARCHAR", ColType::Text),
        DataType::Date32 | DataType::Date64 => ("DATE", ColType::Date),
        DataType::Timestamp(_, tz) => (if tz.is_some() { "TIMESTAMPTZ" } else { "TIMESTAMP" }, ColType::DateTime),
        DataType::Time32(_) | DataType::Time64(_) => ("TIME", ColType::Time),
        DataType::Interval(_) | DataType::Duration(_) => ("INTERVAL", ColType::Text),
        DataType::Binary | DataType::LargeBinary | DataType::FixedSizeBinary(_) | DataType::BinaryView => ("BLOB", ColType::Text),
        DataType::List(_) | DataType::LargeList(_) | DataType::FixedSizeList(..) => ("LIST", ColType::Text),
        DataType::Struct(_) => ("STRUCT", ColType::Text),
        DataType::Map(..) => ("MAP", ColType::Text),
        DataType::Dictionary(..) => ("ENUM", ColType::Text),
        DataType::Null => ("NULL", ColType::Any),
        other => return (format!("{other}"), ColType::Any),
    };
    (n.to_string(), t)
}

/// Runs one statement on a connection.
pub fn run_query(conn: &Connection, sql: &str, cap: usize) -> Result<QueryResult> {
    let sql_run = text::rewrite_top(sql);
    let t0 = Instant::now();
    let mut stmt = match conn.prepare(&sql_run) {
        Ok(s) => s,
        Err(e) => {
            // Some statements can't be prepared (e.g. a few PRAGMAs): run them directly.
            let msg = e.to_string();
            if msg.to_ascii_lowercase().contains("prepare") && conn.execute_batch(&sql_run).is_ok() {
                return Ok(QueryResult { elapsed_ms: t0.elapsed().as_secs_f64() * 1000.0, sql: sql.to_string(), ..QueryResult::default() });
            }
            return Err(crate::friendly_error(&msg));
        }
    };
    let mut rows = stmt.query([]).map_err(err)?;
    let (columns, types, col_types) = match rows.as_ref() {
        Some(st) => {
            let schema = st.schema();
            let mut cols = Vec::new();
            let mut tys = Vec::new();
            let mut cts = Vec::new();
            for f in schema.fields() {
                cols.push(f.name().to_string());
                let (n, t) = type_of(f.data_type());
                tys.push(n);
                cts.push(t);
            }
            (cols, tys, cts)
        }
        None => (vec![], vec![], vec![]),
    };
    let n = columns.len();
    let mut out = Vec::new();
    let mut truncated = false;
    while let Some(r) = rows.next().map_err(err)? {
        if out.len() >= cap {
            truncated = true;
            break;
        }
        let row: Vec<Scalar> = (0..n).map(|i| r.get_ref(i).map(from_value).unwrap_or(Scalar::Null)).collect();
        out.push(row);
    }
    drop(rows);
    let word = text::first_word(sql).to_ascii_uppercase();
    let rows_affected = if matches!(word.as_str(), "INSERT" | "UPDATE" | "DELETE" | "CREATE" | "COPY") && n == 1 && columns.first().is_some_and(|c| c == "Count") {
        out.first().and_then(|r| r.first()).and_then(Scalar::as_f64).map(|c| c as u64)
    } else {
        None
    };
    Ok(QueryResult { columns, types, col_types, rows: out, truncated, elapsed_ms: t0.elapsed().as_secs_f64() * 1000.0, rows_affected, sql: sql.to_string() })
}

pub fn explain(conn: &Connection, sql: &str, analyze: bool) -> Result<String> {
    let inner = sql.trim().trim_end_matches(';');
    let stmt = if text::is_explain(inner) { inner.to_string() } else if analyze { format!("EXPLAIN ANALYZE {inner}") } else { format!("EXPLAIN {inner}") };
    let r = run_query(conn, &stmt, 10_000)?;
    // EXPLAIN returns (explain_key, explain_value); the plan text is in the last column.
    let mut text = String::new();
    for row in &r.rows {
        if let Some(Scalar::Text(t)) = row.last() {
            text.push_str(t);
            text.push('\n');
        }
    }
    Ok(text)
}

pub fn export(conn: &Connection, sql: &str, path: &str) -> Result<()> {
    let lower = path.to_ascii_lowercase();
    let fmt = if lower.ends_with(".parquet") {
        "FORMAT PARQUET"
    } else if lower.ends_with(".json") || lower.ends_with(".jsonl") {
        "FORMAT JSON"
    } else {
        "FORMAT CSV, HEADER"
    };
    let inner = text::rewrite_top(sql.trim().trim_end_matches(';'));
    conn.execute_batch(&format!("COPY ({inner}) TO {} ({fmt})", quote_str(path))).map_err(err)
}

pub fn run_script(conn: &Connection, sql: &str, cap: usize) -> Vec<StatementOutput> {
    let mut out = Vec::new();
    for r in text::split_statements(sql) {
        let stmt = sql.get(r).unwrap_or_default().to_string();
        if text::is_explain(&stmt) {
            let t0 = Instant::now();
            match explain(conn, &stmt, false) {
                Ok(text) => out.push(StatementOutput::Plan { sql: stmt, text, elapsed_ms: t0.elapsed().as_secs_f64() * 1000.0 }),
                Err(message) => {
                    out.push(StatementOutput::Error { sql: stmt, message });
                    break;
                }
            }
            continue;
        }
        match run_query(conn, &stmt, cap) {
            Ok(r) => out.push(StatementOutput::Rows(r)),
            Err(message) => {
                out.push(StatementOutput::Error { sql: stmt, message });
                break;
            }
        }
    }
    out
}
