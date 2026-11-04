//! Query history (`~/.vizardcraft/sql_history.jsonl`) and saved snippets
//! (`~/.vizardcraft/sql_snippets.json`).

use std::io::Write;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct HistoryEntry {
    /// Unix seconds.
    pub at: f64,
    pub sql: String,
    pub ms: f64,
    pub rows: Option<usize>,
    pub error: Option<String>,
    pub project: String,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Snippet {
    pub name: String,
    pub sql: String,
}

pub fn dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("VIZARDCRAFT_HOME") {
        return Some(PathBuf::from(d));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".vizardcraft"))
}

pub fn load_history() -> Vec<HistoryEntry> {
    let Some(p) = dir().map(|d| d.join("sql_history.jsonl")) else { return vec![] };
    let text = std::fs::read_to_string(p).unwrap_or_default();
    let mut v: Vec<HistoryEntry> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    // Newest first, at most 500.
    v.reverse();
    v.truncate(500);
    v
}

pub fn append_history(e: &HistoryEntry) {
    let Some(d) = dir() else { return };
    let _ = std::fs::create_dir_all(&d);
    let path = d.join("sql_history.jsonl");
    // Keep the file from growing without bound.
    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > 4_000_000 {
        let mut all = load_history();
        all.truncate(1000);
        all.reverse();
        let text: String = all.iter().filter_map(|x| serde_json::to_string(x).ok()).map(|l| l + "\n").collect();
        let _ = std::fs::write(&path, text);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path)
        && let Ok(line) = serde_json::to_string(e)
    {
        let _ = writeln!(f, "{line}");
    }
}

pub fn clear_history() {
    if let Some(d) = dir() {
        let _ = std::fs::remove_file(d.join("sql_history.jsonl"));
    }
}

pub fn load_snippets() -> Vec<Snippet> {
    let saved: Option<Vec<Snippet>> = dir().and_then(|d| std::fs::read_to_string(d.join("sql_snippets.json")).ok()).and_then(|s| serde_json::from_str(&s).ok());
    saved.unwrap_or_else(builtin_snippets)
}

pub fn save_snippets(s: &[Snippet]) {
    let Some(d) = dir() else { return };
    let _ = std::fs::create_dir_all(&d);
    if let Ok(text) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write(d.join("sql_snippets.json"), text);
    }
}

pub fn builtin_snippets() -> Vec<Snippet> {
    let s = |name: &str, sql: &str| Snippet { name: name.into(), sql: sql.into() };
    vec![
        s("Explore a table", "SUMMARIZE Sales;"),
        s("Top 10 by a measure", "SELECT ProductID, SUM(Quantity * \"Unit Price\") AS revenue\nFROM Sales\nGROUP BY ALL\nORDER BY revenue DESC\nLIMIT 10;"),
        s("Monthly trend", "SELECT date_trunc('month', \"Date\") AS month, SUM(Quantity) AS qty\nFROM Sales\nGROUP BY ALL\nORDER BY month;"),
        s("Running total (window)", "SELECT \"Date\", SUM(Quantity) AS qty,\n       SUM(SUM(Quantity)) OVER (ORDER BY \"Date\") AS running_qty\nFROM Sales\nGROUP BY ALL\nORDER BY \"Date\";"),
        s("Pivot", "PIVOT Sales ON Channel USING SUM(Quantity) GROUP BY StoreID;"),
        s("Save to the warehouse", "CREATE OR REPLACE TABLE sales_by_store AS\nSELECT StoreID, SUM(Quantity) AS qty FROM Sales GROUP BY ALL;"),
        s("Read a CSV file", "SELECT * FROM read_csv_auto('/path/to/file.csv') LIMIT 100;"),
        s("Read Parquet files", "SELECT * FROM read_parquet('/path/to/*.parquet') LIMIT 100;"),
        s("Attach SQLite", "INSTALL sqlite; LOAD sqlite;\nATTACH '/path/to/db.sqlite' AS lite (TYPE SQLITE, READ_ONLY);\nSHOW ALL TABLES;"),
        s("Attach PostgreSQL", "INSTALL postgres; LOAD postgres;\nATTACH 'host=localhost dbname=mydb user=me' AS pg (TYPE POSTGRES, READ_ONLY);\nSHOW ALL TABLES;"),
        s("Export to Parquet", "COPY (SELECT * FROM Sales) TO '/path/to/sales.parquet' (FORMAT PARQUET);"),
        s("Query plan", "EXPLAIN ANALYZE SELECT StoreID, SUM(Quantity) FROM Sales GROUP BY ALL;"),
    ]
}
