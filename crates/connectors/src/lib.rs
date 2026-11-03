//! The desktop host for Power Query: files, folders, Excel workbooks, the web and databases.

use vizcraft_query::m::eval::Host;
use vizcraft_query::{Error, Result, Table, Value};

/// Reads files from disk; the other connectors are added per source type.
#[derive(Default)]
pub struct DesktopHost;

impl Host for DesktopHost {
    fn file_contents(&self, path: &str) -> Result<Vec<u8>> {
        std::fs::read(path).map_err(|e| Error::Eval(format!("DataSource.Error: Could not find file '{path}': {e}")))
    }
    fn folder_files(&self, path: &str) -> Result<Table> {
        let rd = std::fs::read_dir(path).map_err(|e| Error::Eval(format!("DataSource.Error: {path}: {e}")))?;
        let mut rows = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let ext = p.extension().map(|x| format!(".{}", x.to_string_lossy())).unwrap_or_default();
            let content = std::fs::read(&p).unwrap_or_default();
            rows.push(vec![Value::Binary(std::rc::Rc::new(content)), Value::text(&name), Value::text(&ext), Value::Null, Value::text(path)]);
        }
        Ok(Table::new(vec!["Content".into(), "Name".into(), "Extension".into(), "Date modified".into(), "Folder Path".into()], rows))
    }
}
