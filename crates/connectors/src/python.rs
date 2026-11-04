//! `Python.Execute(script, [dataset = table])`: runs a Python script (the interpreter in
//! `VIZARDCRAFT_PYTHON`, else `python3`). Tables passed in become pandas DataFrames (or lists of
//! dicts without pandas); every DataFrame (or list of dicts) the script leaves in a variable
//! comes back as a row `Name`, `Value` — as in Power BI.

use std::process::Command;

use vizardcraft_query::funcs::source::csv_table;
use vizardcraft_query::{Error, MType, Result, Table, Value};

use crate::Args;

const PRELUDE: &str = r#"
import csv, os, sys
try:
    import pandas as pd
except Exception:
    pd = None
def _vc_read(path):
    if pd is not None:
        return pd.read_csv(path)
    with open(path, newline='', encoding='utf-8') as f:
        return list(csv.DictReader(f))
"#;

const EPILOGUE: &str = r#"
_vc_out = os.environ.get('VC_OUT', '.')
_vc_names = []
for _k, _v in list(globals().items()):
    if _k.startswith('_'):
        continue
    try:
        if pd is not None and isinstance(_v, pd.DataFrame):
            _v.to_csv(os.path.join(_vc_out, _k + '.csv'), index=False)
            _vc_names.append(_k)
        elif isinstance(_v, list) and _v and all(isinstance(r, dict) for r in _v):
            _cols = []
            for r in _v:
                for c in r:
                    if c not in _cols:
                        _cols.append(c)
            with open(os.path.join(_vc_out, _k + '.csv'), 'w', newline='', encoding='utf-8') as f:
                w = csv.DictWriter(f, fieldnames=_cols)
                w.writeheader()
                w.writerows(_v)
            _vc_names.append(_k)
    except Exception as e:
        print(e, file=sys.stderr)
with open(os.path.join(_vc_out, '_names.txt'), 'w', encoding='utf-8') as f:
    f.write('\n'.join(_vc_names))
"#;

pub fn execute(a: &Args) -> Result<Table> {
    let script = a.pos(0).unwrap_or_default();
    let dir = std::env::temp_dir().join(format!("vizardcraft-py-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)));
    std::fs::create_dir_all(&dir).map_err(|e| Error::Eval(format!("Python: {e}")))?;
    let mut src = String::from(PRELUDE);
    for (k, csv) in &a.options {
        let ident: String = k.chars().filter(|c| c.is_alphanumeric() || *c == '_').collect();
        if ident.is_empty() {
            continue;
        }
        let path = dir.join(format!("in_{ident}.csv"));
        std::fs::write(&path, csv).map_err(|e| Error::Eval(format!("Python: {e}")))?;
        src.push_str(&format!("{ident} = _vc_read({:?})\n", path.to_string_lossy()));
    }
    src.push_str(script);
    src.push('\n');
    src.push_str(EPILOGUE);
    let file = dir.join("script.py");
    std::fs::write(&file, src).map_err(|e| Error::Eval(format!("Python: {e}")))?;
    let exe = std::env::var("VIZARDCRAFT_PYTHON").unwrap_or_else(|_| "python3".into());
    let out = Command::new(&exe).arg(&file).env("VC_OUT", &dir).current_dir(&dir).output().map_err(|e| Error::Eval(format!("DataSource.Error: Python couldn't start ({exe}): {e}")))?;
    if !out.status.success() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(Error::Eval(format!("DataSource.Error: The Python script failed: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    let names = std::fs::read_to_string(dir.join("_names.txt")).unwrap_or_default();
    let mut rows = Vec::new();
    for n in names.lines().filter(|n| !n.is_empty()) {
        let text = std::fs::read_to_string(dir.join(format!("{n}.csv"))).unwrap_or_default();
        let t = crate::promote_headers(csv_table(&text, ',', true, None));
        rows.push(vec![Value::text(n), Value::table(retype(t))]);
    }
    let _ = std::fs::remove_dir_all(&dir);
    let mut t = Table::new(vec!["Name".into(), "Value".into()], rows);
    t.types = vec![MType::Text, MType::Table];
    Ok(t)
}

/// Numbers in CSV text back to numbers.
fn retype(mut t: Table) -> Table {
    for c in 0..t.columns.len() {
        let numeric = t.rows.iter().all(|r| match r.get(c) {
            Some(Value::Text(s)) => s.is_empty() || s.parse::<f64>().is_ok(),
            _ => true,
        });
        if numeric {
            for r in t.rows.iter_mut() {
                if let Some(v) = r.get_mut(c) {
                    *v = match &*v {
                        Value::Text(s) if s.is_empty() => Value::Null,
                        Value::Text(s) => s.parse::<f64>().map(Value::Number).unwrap_or(Value::Null),
                        x => x.clone(),
                    };
                }
            }
        }
    }
    crate::infer_types(&mut t);
    t
}
