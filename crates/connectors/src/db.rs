//! Databases: SQLite (bundled), PostgreSQL and MySQL (pure-Rust drivers, no TLS), and the
//! connectors not available yet (SQL Server, ODBC).
//!
//! Without a native query each returns a navigation table (`Name`, `Data`, `Schema`, `Item`,
//! `Kind`) whose `Data` holds each table's rows (at most [`NAV_ROWS`] per table: use a native
//! query, or the Get Data navigator, which writes one, for bigger tables). With `[Query="…"]`
//! the query's result is returned.
//!
//! Credentials come from the function's options (`[Username="…", Password="…"]`), from a
//! connection URL given as the server (`postgresql://user:pass@host/db`), or from the
//! environment (`PGUSER`/`PGPASSWORD`, `MYSQL_USER`/`MYSQL_PASSWORD`).

use vizardcraft_query::value::{days_of, serial_of};
use vizardcraft_query::{Error, MType, Result, Table, Value};

use crate::Args;

/// Rows per table in a navigation table.
pub const NAV_ROWS: usize = 100_000;
/// Rows a native query returns at most.
pub const MAX_ROWS: usize = 5_000_000;
/// Tables a navigation table lists at most.
const NAV_TABLES: usize = 300;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Number,
    Logical,
    Date,
    DateTime,
    Time,
    Text,
}

impl Kind {
    fn mtype(self) -> MType {
        match self {
            Kind::Number => MType::Number,
            Kind::Logical => MType::Logical,
            Kind::Date => MType::Date,
            Kind::DateTime => MType::DateTime,
            Kind::Time => MType::Time,
            Kind::Text => MType::Text,
        }
    }
}

/// A database text value as M, by the column's kind.
pub fn typed(s: &str, k: Kind) -> Value {
    match k {
        Kind::Number => s.trim().parse::<f64>().map(Value::Number).unwrap_or_else(|_| Value::text(s)),
        Kind::Logical => Value::Logical(matches!(s.trim(), "t" | "true" | "1" | "TRUE" | "True")),
        Kind::Date => chrono::NaiveDate::parse_from_str(s.get(..10).unwrap_or(s), "%Y-%m-%d").map(|d| Value::Date(days_of(d))).unwrap_or_else(|_| Value::text(s)),
        Kind::DateTime => chrono::NaiveDateTime::parse_from_str(s.get(..19).unwrap_or(s), "%Y-%m-%d %H:%M:%S")
            .or_else(|_| chrono::NaiveDateTime::parse_from_str(s.get(..19).unwrap_or(s), "%Y-%m-%dT%H:%M:%S"))
            .map(|d| Value::DateTime(serial_of(d)))
            .unwrap_or_else(|_| Value::text(s)),
        Kind::Time => chrono::NaiveTime::parse_from_str(s.get(..8).unwrap_or(s), "%H:%M:%S")
            .map(|t| Value::Time(chrono::Timelike::num_seconds_from_midnight(&t) as f64 / 86400.0))
            .unwrap_or_else(|_| Value::text(s)),
        Kind::Text => Value::text(s),
    }
}

/// The kind of a column from its SQL type name.
pub fn kind_of(type_name: &str) -> Kind {
    let t = type_name.to_ascii_lowercase();
    if t.starts_with("bool") || t == "bit" {
        Kind::Logical
    } else if t.contains("int") || t.contains("float") || t.contains("double") || t.contains("real") || t.contains("numeric") || t.contains("decimal") || t == "money" || t == "oid" || t == "year" {
        Kind::Number
    } else if t.starts_with("timestamp") || t.starts_with("datetime") {
        Kind::DateTime
    } else if t == "date" || t == "newdate" {
        Kind::Date
    } else if t.starts_with("time") {
        Kind::Time
    } else {
        Kind::Text
    }
}

fn nav_table(rows: Vec<(String, String, String, Value)>) -> Table {
    let rows = rows.into_iter().map(|(schema, item, kind, data)| {
        let name = if schema.is_empty() || schema == "main" { item.clone() } else { format!("{schema}.{item}") };
        vec![Value::text(&name), data, Value::text(&schema), Value::text(&item), Value::text(&kind)]
    });
    let mut t = Table::new(["Name", "Data", "Schema", "Item", "Kind"].iter().map(|s| s.to_string()).collect(), rows.collect());
    t.types = vec![MType::Text, MType::Table, MType::Text, MType::Text, MType::Text];
    t
}

/// Quotes an SQL identifier with double quotes (or backticks for MySQL).
pub fn quote_ident(name: &str, backtick: bool) -> String {
    if backtick { format!("`{}`", name.replace('`', "``")) } else { format!("\"{}\"", name.replace('"', "\"\"")) }
}

// ---------------------------------------------------------------- SQLite

fn sqlite_value(v: rusqlite::types::ValueRef<'_>) -> Value {
    use rusqlite::types::ValueRef;
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Number(i as f64),
        ValueRef::Real(f) => Value::Number(f),
        ValueRef::Text(t) => Value::text(String::from_utf8_lossy(t)),
        ValueRef::Blob(b) => Value::Binary(std::rc::Rc::new(b.to_vec())),
    }
}

fn sqlite_run(conn: &rusqlite::Connection, sql: &str, limit: usize) -> Result<Table> {
    let e = |e: rusqlite::Error| Error::Eval(format!("DataSource.Error: SQLite: {e}"));
    let mut stmt = conn.prepare(sql).map_err(e)?;
    let cols: Vec<String> = stmt.column_names().map(|s| s.to_string()).collect();
    let decl: Vec<Option<String>> = stmt.columns().map(|c| c.decl_type().map(str::to_string)).collect();
    let n = cols.len();
    let mut rows = Vec::new();
    let mut q = stmt.query([]).map_err(e)?;
    while let Some(r) = q.next().map_err(e)? {
        if rows.len() >= limit {
            break;
        }
        let row: Vec<Value> = (0..n).map(|i| r.get_ref(i).map(sqlite_value).unwrap_or_default()).collect();
        rows.push(row);
    }
    let mut t = Table::new(cols, rows);
    crate::infer_types(&mut t);
    // Declared date columns hold text; type them.
    for (i, d) in decl.iter().enumerate() {
        let k = d.as_deref().map(kind_of).unwrap_or(Kind::Text);
        if matches!(k, Kind::Date | Kind::DateTime) {
            for r in t.rows.iter_mut() {
                if let Some(v @ Value::Text(_)) = r.get_mut(i) {
                    let s = v.display();
                    *v = typed(&s, k);
                }
            }
            if let Some(ty) = t.types.get_mut(i) {
                *ty = k.mtype();
            }
        }
    }
    Ok(t)
}

/// `Sqlite.Database(path, [Query="…"])`.
pub fn sqlite(a: &Args, query: Option<&str>) -> Result<Table> {
    let Some(path) = a.pos(0) else { return crate::err("Sqlite.Database needs the database file's path.") };
    if !std::path::Path::new(path).exists() {
        return Err(Error::Eval(format!("DataSource.NotFound: The SQLite database '{path}' doesn't exist.")));
    }
    let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| Error::Eval(format!("DataSource.Error: SQLite: {e}")))?;
    if let Some(q) = query.or(a.opt("Query")) {
        return sqlite_run(&conn, q, MAX_ROWS);
    }
    let list = sqlite_run(&conn, "SELECT name, type FROM sqlite_master WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name", NAV_TABLES)?;
    let mut rows = Vec::new();
    for r in &list.rows {
        let name = r.first().map(Value::display).unwrap_or_default();
        let kind = if r.get(1).map(Value::display).as_deref() == Some("view") { "View" } else { "Table" };
        let data = match sqlite_run(&conn, &format!("SELECT * FROM {}", quote_ident(&name, false)), NAV_ROWS) {
            Ok(t) => Value::table(t),
            Err(e) => Value::error(e.message()),
        };
        rows.push((String::new(), name, kind.to_string(), data));
    }
    Ok(nav_table(rows))
}

/// The tables of a SQLite file (for the navigator), without reading their rows.
pub fn sqlite_tables(path: &str) -> Result<Vec<(String, String)>> {
    let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| Error::Eval(format!("DataSource.Error: SQLite: {e}")))?;
    let t = sqlite_run(&conn, "SELECT name, type FROM sqlite_master WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name", NAV_TABLES)?;
    Ok(t.rows.iter().map(|r| (r.first().map(Value::display).unwrap_or_default(), r.get(1).map(Value::display).unwrap_or_default())).collect())
}

// ---------------------------------------------------------------- server connection details

/// Host, port, database, user and password for a server database.
#[derive(Clone, Debug, Default)]
pub struct Conn {
    pub host: String,
    pub port: Option<u16>,
    pub database: String,
    pub user: Option<String>,
    pub password: Option<String>,
}

pub fn parse_conn(a: &Args, user_env: &str, pass_env: &str) -> Conn {
    let server = a.pos(0).unwrap_or("localhost").trim();
    let mut c = Conn { database: a.pos(1).unwrap_or_default().to_string(), ..Default::default() };
    // `scheme://user:pass@host:port/db`
    let rest = server.split_once("://").map(|(_, r)| r).unwrap_or(server);
    let (cred, hostpart) = match rest.rsplit_once('@') {
        Some((c, h)) => (Some(c), h),
        None => (None, rest),
    };
    if let Some(cred) = cred {
        let (u, p) = cred.split_once(':').unwrap_or((cred, ""));
        c.user = Some(u.to_string());
        if !p.is_empty() {
            c.password = Some(p.to_string());
        }
    }
    let (hp, db) = hostpart.split_once('/').unwrap_or((hostpart, ""));
    if c.database.is_empty() {
        c.database = db.to_string();
    }
    let (h, p) = match hp.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|x| x.is_ascii_digit()) => (h, p.parse().ok()),
        _ => (hp, None),
    };
    c.host = if h.is_empty() { "localhost".into() } else { h.to_string() };
    c.port = p;
    if let Some(u) = a.opt("Username").or(a.opt("User")) {
        c.user = Some(u.to_string());
    }
    if let Some(p) = a.opt("Password") {
        c.password = Some(p.to_string());
    }
    if c.user.is_none() {
        c.user = std::env::var(user_env).ok();
    }
    if c.password.is_none() {
        c.password = std::env::var(pass_env).ok();
    }
    c
}

// ---------------------------------------------------------------- PostgreSQL

fn pg_err(e: postgres::Error) -> Error {
    Error::Eval(format!("DataSource.Error: PostgreSQL: {e}"))
}

fn pg_run(client: &mut postgres::Client, sql: &str, limit: usize) -> Result<Table> {
    // The statement gives the column types; the simple protocol gives every value as text.
    let kinds: Vec<Kind> = client.prepare(sql).map(|s| s.columns().iter().map(|c| kind_of(c.type_().name())).collect()).unwrap_or_default();
    let msgs = client.simple_query(sql).map_err(pg_err)?;
    let mut cols: Vec<String> = Vec::new();
    let mut rows = Vec::new();
    for m in msgs {
        match m {
            postgres::SimpleQueryMessage::RowDescription(d) => {
                cols = d.iter().map(|c| c.name().to_string()).collect();
            }
            postgres::SimpleQueryMessage::Row(r) => {
                if cols.is_empty() {
                    cols = r.columns().iter().map(|c| c.name().to_string()).collect();
                }
                if rows.len() < limit {
                    rows.push(
                        (0..r.len())
                            .map(|i| match r.get(i) {
                                Some(s) => typed(s, kinds.get(i).copied().unwrap_or(Kind::Text)),
                                None => Value::Null,
                            })
                            .collect(),
                    );
                }
            }
            _ => {}
        }
    }
    let mut t = Table::new(cols, rows);
    if kinds.len() == t.columns.len() {
        t.types = kinds.iter().map(|k| k.mtype()).collect();
    } else {
        crate::infer_types(&mut t);
    }
    Ok(t)
}

/// `PostgreSQL.Database(server, database, [Query="…"])`.
pub fn postgres(a: &Args, query: Option<&str>) -> Result<Table> {
    let c = parse_conn(a, "PGUSER", "PGPASSWORD");
    let mut cfg = postgres::Config::new();
    cfg.host(&c.host).port(c.port.unwrap_or(5432)).user(c.user.as_deref().unwrap_or("postgres"));
    cfg.connect_timeout(std::time::Duration::from_secs(15));
    if !c.database.is_empty() {
        cfg.dbname(&c.database);
    }
    if let Some(p) = &c.password {
        cfg.password(p);
    }
    let mut client = cfg.connect(postgres::NoTls).map_err(pg_err)?;
    if let Some(q) = query.or(a.opt("Query")) {
        return pg_run(&mut client, q, MAX_ROWS);
    }
    let list = pg_run(
        &mut client,
        "SELECT table_schema, table_name, table_type FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog','information_schema') ORDER BY 1, 2",
        NAV_TABLES,
    )?;
    let mut rows = Vec::new();
    for r in &list.rows {
        let schema = r.first().map(Value::display).unwrap_or_default();
        let name = r.get(1).map(Value::display).unwrap_or_default();
        let kind = if r.get(2).map(Value::display).is_some_and(|k| k.contains("VIEW")) { "View" } else { "Table" };
        let sql = format!("SELECT * FROM {}.{} LIMIT {NAV_ROWS}", quote_ident(&schema, false), quote_ident(&name, false));
        let data = pg_run(&mut client, &sql, NAV_ROWS).map(Value::table).unwrap_or_else(|e| Value::error(e.message()));
        rows.push((schema, name, kind.to_string(), data));
    }
    Ok(nav_table(rows))
}

// ---------------------------------------------------------------- MySQL

fn my_err(e: mysql::Error) -> Error {
    Error::Eval(format!("DataSource.Error: MySQL: {e}"))
}

fn my_kind(t: mysql::consts::ColumnType) -> Kind {
    use mysql::consts::ColumnType as T;
    match t {
        T::MYSQL_TYPE_TINY | T::MYSQL_TYPE_SHORT | T::MYSQL_TYPE_LONG | T::MYSQL_TYPE_LONGLONG | T::MYSQL_TYPE_INT24 | T::MYSQL_TYPE_FLOAT | T::MYSQL_TYPE_DOUBLE
        | T::MYSQL_TYPE_DECIMAL | T::MYSQL_TYPE_NEWDECIMAL | T::MYSQL_TYPE_YEAR => Kind::Number,
        T::MYSQL_TYPE_DATE | T::MYSQL_TYPE_NEWDATE => Kind::Date,
        T::MYSQL_TYPE_DATETIME | T::MYSQL_TYPE_TIMESTAMP | T::MYSQL_TYPE_DATETIME2 | T::MYSQL_TYPE_TIMESTAMP2 => Kind::DateTime,
        T::MYSQL_TYPE_TIME | T::MYSQL_TYPE_TIME2 => Kind::Time,
        T::MYSQL_TYPE_BIT => Kind::Logical,
        _ => Kind::Text,
    }
}

fn my_value(v: &mysql::Value, k: Kind) -> Value {
    match v {
        mysql::Value::NULL => Value::Null,
        mysql::Value::Bytes(b) => typed(&String::from_utf8_lossy(b), k),
        mysql::Value::Int(i) => Value::Number(*i as f64),
        mysql::Value::UInt(i) => Value::Number(*i as f64),
        mysql::Value::Float(f) => Value::Number(*f as f64),
        mysql::Value::Double(f) => Value::Number(*f),
        mysql::Value::Date(y, mo, d, h, mi, s, _) => {
            match chrono::NaiveDate::from_ymd_opt(*y as i32, *mo as u32, *d as u32).and_then(|d0| d0.and_hms_opt(*h as u32, *mi as u32, *s as u32)) {
                Some(dt) if k == Kind::Date => Value::Date(days_of(dt.date())),
                Some(dt) => Value::DateTime(serial_of(dt)),
                None => Value::Null,
            }
        }
        mysql::Value::Time(neg, d, h, mi, s, _) => {
            let x = *d as f64 + (*h as f64 * 3600.0 + *mi as f64 * 60.0 + *s as f64) / 86400.0;
            Value::Duration(if *neg { -x } else { x })
        }
    }
}

fn my_run(conn: &mut mysql::Conn, sql: &str, limit: usize) -> Result<Table> {
    use mysql::prelude::Queryable;
    let mut res = conn.query_iter(sql).map_err(my_err)?;
    let mut cols: Vec<String> = Vec::new();
    let mut kinds: Vec<Kind> = Vec::new();
    let mut rows = Vec::new();
    if let Some(set) = res.iter() {
        for c in set.columns().as_ref() {
            cols.push(c.name_str().to_string());
            kinds.push(my_kind(c.column_type()));
        }
        for r in set {
            let r = r.map_err(my_err)?;
            if rows.len() >= limit {
                continue;
            }
            let vals = r.unwrap();
            rows.push(vals.iter().enumerate().map(|(i, v)| my_value(v, kinds.get(i).copied().unwrap_or(Kind::Text))).collect());
        }
    }
    let mut t = Table::new(cols, rows);
    t.types = kinds.iter().map(|k| k.mtype()).collect();
    Ok(t)
}

/// `MySQL.Database(server, database, [Query="…"])`.
pub fn mysql(a: &Args, query: Option<&str>) -> Result<Table> {
    let c = parse_conn(a, "MYSQL_USER", "MYSQL_PASSWORD");
    let opts = mysql::OptsBuilder::new()
        .ip_or_hostname(Some(c.host.clone()))
        .tcp_port(c.port.unwrap_or(3306))
        .user(Some(c.user.clone().unwrap_or_else(|| "root".into())))
        .pass(c.password.clone())
        .db_name(if c.database.is_empty() { None } else { Some(c.database.clone()) })
        .tcp_connect_timeout(Some(std::time::Duration::from_secs(15)));
    let mut conn = mysql::Conn::new(opts).map_err(my_err)?;
    if let Some(q) = query.or(a.opt("Query")) {
        return my_run(&mut conn, q, MAX_ROWS);
    }
    let list = my_run(
        &mut conn,
        "SELECT table_schema, table_name, table_type FROM information_schema.tables WHERE table_schema = DATABASE() OR (DATABASE() IS NULL AND table_schema NOT IN ('mysql','information_schema','performance_schema','sys')) ORDER BY 1, 2",
        NAV_TABLES,
    )?;
    let mut rows = Vec::new();
    for r in &list.rows {
        let schema = r.first().map(Value::display).unwrap_or_default();
        let name = r.get(1).map(Value::display).unwrap_or_default();
        let kind = if r.get(2).map(Value::display).is_some_and(|k| k.contains("VIEW")) { "View" } else { "Table" };
        let sql = format!("SELECT * FROM {}.{} LIMIT {NAV_ROWS}", quote_ident(&schema, true), quote_ident(&name, true));
        let data = my_run(&mut conn, &sql, NAV_ROWS).map(Value::table).unwrap_or_else(|e| Value::error(e.message()));
        rows.push((schema, name, kind.to_string(), data));
    }
    Ok(nav_table(rows))
}

// ---------------------------------------------------------------- not yet

pub fn sql_server(a: &Args, _query: Option<&str>) -> Result<Table> {
    Err(Error::Eval(format!(
        "DataSource.Error: The SQL Server connector isn't available in this build yet (server '{}'). Export the data to CSV or Parquet, or connect through PostgreSQL, MySQL or SQLite.",
        a.pos(0).unwrap_or_default()
    )))
}

pub fn odbc(_a: &Args, _query: Option<&str>) -> Result<Table> {
    Err(Error::Eval("DataSource.Error: ODBC isn't available in this build yet: it needs the system's ODBC driver manager.".into()))
}
