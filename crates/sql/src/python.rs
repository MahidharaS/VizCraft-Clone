//! The Python bridge: runs `python3` with a query result as a pandas DataFrame `df`; whatever
//! the script leaves in `out` (a DataFrame, Series or anything `pd.DataFrame()` accepts) comes
//! back as a result grid. Data goes through temp files (CSV in, JSON out), so only pandas is
//! needed — no DuckDB or Arrow Python packages.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use vizardcraft_core::build::ColType;
use vizardcraft_core::scalar::Scalar;

use crate::{QueryResult, Result};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The starter script shown in a new Python tab.
pub const TEMPLATE: &str = "# `df` is the last SQL result as a pandas DataFrame.\n# Put a DataFrame in `out` to see it as a grid.\nprint(df.describe(include='all'))\nout = df.head(20)\n";

#[derive(Clone, Debug, Default)]
pub struct PythonOutput {
    pub stdout: String,
    pub stderr: String,
    pub out: Option<QueryResult>,
    pub ok: bool,
    pub elapsed_ms: f64,
}

pub struct PythonRun {
    child: Child,
    dir: PathBuf,
    started: Instant,
}

/// The Python interpreter to use: `$VIZARDCRAFT_PYTHON`, `python3`, then `python`.
pub fn interpreter() -> Option<String> {
    let mut cands = vec![];
    if let Ok(p) = std::env::var("VIZARDCRAFT_PYTHON") {
        cands.push(p);
    }
    cands.push("python3".into());
    cands.push("python".into());
    cands.into_iter().find(|c| Command::new(c).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success()))
}

const WRAPPER: &str = r#"
import sys, json
try:
    import pandas as pd
except ImportError:
    sys.stderr.write("VIZARDCRAFT_NO_PANDAS\n")
    sys.exit(3)
_cfg = json.load(open(sys.argv[1]))
try:
    df = pd.read_csv(_cfg["input"], parse_dates=_cfg["dates"]) if _cfg["has_columns"] else pd.DataFrame()
except Exception:
    df = pd.read_csv(_cfg["input"]) if _cfg["has_columns"] else pd.DataFrame()
out = None
exec(compile(open(_cfg["script"]).read(), "<python cell>", "exec"), globals())
if out is not None:
    if isinstance(out, pd.Series):
        out = out.to_frame()
    if not isinstance(out, pd.DataFrame):
        out = pd.DataFrame(out)
    if not isinstance(out.index, pd.RangeIndex):
        out = out.reset_index()
    payload = {
        "columns": [str(c) for c in out.columns],
        "dtypes": [str(t) for t in out.dtypes],
        "data": json.loads(out.to_json(orient="values", date_format="iso", date_unit="s")),
    }
    with open(_cfg["output"], "w") as f:
        json.dump(payload, f)
"#;

/// Starts a script over `df`.
pub fn start(script: &str, df: &QueryResult) -> Result<PythonRun> {
    let py = interpreter().ok_or("Python isn't installed (or not on PATH). Install Python 3 with pandas (`pip install pandas`), or set VIZARDCRAFT_PYTHON to its path.")?;
    let dir = std::env::temp_dir().join(format!("vizardcraft-py-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let input = dir.join("df.csv");
    let user = dir.join("cell.py");
    let wrapper = dir.join("wrapper.py");
    let output = dir.join("out.json");
    let cfg = dir.join("cfg.json");
    std::fs::write(&input, df.to_csv()).map_err(|e| e.to_string())?;
    std::fs::write(&user, script).map_err(|e| e.to_string())?;
    std::fs::write(&wrapper, WRAPPER).map_err(|e| e.to_string())?;
    let dates: Vec<&String> = df.columns.iter().zip(&df.col_types).filter(|(_, t)| matches!(t, ColType::Date | ColType::DateTime)).map(|(c, _)| c).collect();
    let conf = serde_json::json!({
        "input": input.to_string_lossy(),
        "script": user.to_string_lossy(),
        "output": output.to_string_lossy(),
        "dates": dates,
        "has_columns": !df.columns.is_empty(),
    });
    std::fs::write(&cfg, conf.to_string()).map_err(|e| e.to_string())?;
    let stdout = std::fs::File::create(dir.join("stdout.txt")).map_err(|e| e.to_string())?;
    let stderr = std::fs::File::create(dir.join("stderr.txt")).map_err(|e| e.to_string())?;
    let child = Command::new(py)
        .arg(&wrapper)
        .arg(&cfg)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .map_err(|e| format!("Couldn't start Python: {e}"))?;
    Ok(PythonRun { child, dir, started: Instant::now() })
}

impl PythonRun {
    /// The output once the process has finished.
    pub fn try_finish(&mut self) -> Option<PythonOutput> {
        let status = match self.child.try_wait() {
            Ok(Some(s)) => s,
            Ok(None) => return None,
            Err(e) => return Some(PythonOutput { stderr: e.to_string(), ..PythonOutput::default() }),
        };
        let stdout = std::fs::read_to_string(self.dir.join("stdout.txt")).unwrap_or_default();
        let mut stderr = std::fs::read_to_string(self.dir.join("stderr.txt")).unwrap_or_default();
        if stderr.contains("VIZARDCRAFT_NO_PANDAS") {
            stderr = "pandas isn't installed for this Python. Install it with `python3 -m pip install pandas`.".into();
        }
        let out = std::fs::read_to_string(self.dir.join("out.json")).ok().and_then(|s| parse_output(&s));
        let _ = std::fs::remove_dir_all(&self.dir);
        Some(PythonOutput { stdout, stderr, out, ok: status.success(), elapsed_ms: self.started.elapsed().as_secs_f64() * 1000.0 })
    }

    pub fn elapsed_secs(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Runs a script and waits for it.
pub fn run_blocking(script: &str, df: &QueryResult) -> Result<PythonOutput> {
    let mut run = start(script, df)?;
    loop {
        if let Some(o) = run.try_finish() {
            return Ok(o);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn parse_output(s: &str) -> Option<QueryResult> {
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    let columns: Vec<String> = v.get("columns")?.as_array()?.iter().map(|c| c.as_str().unwrap_or_default().to_string()).collect();
    let dtypes: Vec<String> = v.get("dtypes")?.as_array()?.iter().map(|c| c.as_str().unwrap_or_default().to_string()).collect();
    let data = v.get("data")?.as_array()?;
    let mut col_types: Vec<ColType> = dtypes
        .iter()
        .map(|d| {
            if d.starts_with("int") || d.starts_with("uint") || d.starts_with("Int") {
                ColType::Integer
            } else if d.starts_with("float") || d.starts_with("Float") {
                ColType::Number
            } else if d == "bool" || d == "boolean" {
                ColType::Bool
            } else if d.starts_with("datetime") {
                ColType::DateTime
            } else if d == "object" || d == "string" || d == "category" {
                ColType::Text
            } else {
                ColType::Any
            }
        })
        .collect();
    col_types.resize(columns.len(), ColType::Any);
    let rows: Vec<Vec<Scalar>> = data
        .iter()
        .map(|r| {
            let cells = r.as_array().cloned().unwrap_or_default();
            (0..columns.len())
                .map(|i| match (col_types.get(i).copied().unwrap_or_default(), cells.get(i).unwrap_or(&serde_json::Value::Null)) {
                    (_, serde_json::Value::Null) => Scalar::Null,
                    (ColType::DateTime, serde_json::Value::String(s)) => vizardcraft_core::build::parse_date(&s.replace('T', " ")).map(Scalar::Num).unwrap_or_else(|| Scalar::Text(s.clone())),
                    (_, serde_json::Value::Bool(b)) => Scalar::Bool(*b),
                    (_, serde_json::Value::Number(n)) => Scalar::Num(n.as_f64().unwrap_or(f64::NAN)),
                    (_, serde_json::Value::String(s)) => Scalar::Text(s.clone()),
                    (_, other) => Scalar::Text(other.to_string()),
                })
                .collect()
        })
        .collect();
    // Date/time columns that are all midnight are dates.
    for (c, ty) in col_types.iter_mut().enumerate() {
        if *ty == ColType::DateTime && rows.iter().all(|r| matches!(r.get(c), Some(Scalar::Num(n)) if n.fract() == 0.0) || matches!(r.get(c), Some(Scalar::Null))) {
            *ty = ColType::Date;
        }
    }
    let types = col_types.iter().map(|t| crate::sql_type(*t).to_string()).collect();
    Some(QueryResult { columns, types, col_types, rows, ..QueryResult::default() })
}
