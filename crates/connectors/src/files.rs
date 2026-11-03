//! Files and folders.

use std::path::Path;
use std::rc::Rc;

use vizcraft_query::{Error, MType, Result, Table, Value};

/// Folder.Files reads at most this many files.
const MAX_FILES: usize = 5000;

pub fn read(path: &str) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| Error::Eval(format!("DataSource.Error: Could not find file '{path}': {e}")))
}

fn serial(t: std::io::Result<std::time::SystemTime>) -> Value {
    let Ok(t) = t else { return Value::Null };
    let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) else { return Value::Null };
    // 1970-01-01 is serial 25569.
    Value::DateTime(25569.0 + d.as_secs_f64() / 86400.0)
}

/// `Folder.Files`: every file in the folder and its subfolders.
pub fn folder(path: &str) -> Result<Table> {
    let root = Path::new(path);
    if !root.is_dir() {
        return Err(Error::Eval(format!("DataSource.NotFound: The folder '{path}' doesn't exist.")));
    }
    let mut rows = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                if depth < 12 {
                    stack.push((p, depth + 1));
                }
                continue;
            }
            if rows.len() >= MAX_FILES {
                break;
            }
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if name.starts_with("~$") {
                continue;
            }
            let ext = p.extension().map(|x| format!(".{}", x.to_string_lossy())).unwrap_or_default();
            let meta = std::fs::metadata(&p).ok();
            let content = std::fs::read(&p).unwrap_or_default();
            let mut folder = p.parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
            if !folder.ends_with(std::path::MAIN_SEPARATOR) {
                folder.push(std::path::MAIN_SEPARATOR);
            }
            rows.push(vec![
                Value::Binary(Rc::new(content)),
                Value::text(&name),
                Value::text(&ext),
                meta.as_ref().map(|m| serial(m.accessed())).unwrap_or_default(),
                meta.as_ref().map(|m| serial(m.modified())).unwrap_or_default(),
                meta.as_ref().map(|m| serial(m.created())).unwrap_or_default(),
                meta.as_ref().map(|m| Value::Number(m.len() as f64)).unwrap_or_default(),
                Value::text(&folder),
            ]);
        }
    }
    let cols = ["Content", "Name", "Extension", "Date accessed", "Date modified", "Date created", "Size", "Folder Path"];
    let mut t = Table::new(cols.iter().map(|s| s.to_string()).collect(), rows);
    t.types = vec![MType::Binary, MType::Text, MType::Text, MType::DateTime, MType::DateTime, MType::DateTime, MType::Number, MType::Text];
    Ok(t)
}
