//! `Excel.Workbook`: sheets and tables of xlsx, xlsm, xlsb, xls and ods files (calamine).

use std::io::Cursor;

use calamine::{Data, Range, Reader, SheetVisible, Sheets, open_workbook_auto_from_rs};
use vizcraft_query::{Error, MType, Result, Table, Value};

fn cell(d: &Data) -> Value {
    match d {
        Data::Int(i) => Value::Number(*i as f64),
        Data::Float(f) => Value::Number(*f),
        Data::String(s) => Value::text(s),
        Data::Bool(b) => Value::Logical(*b),
        Data::DateTime(dt) => {
            if dt.is_duration() {
                return Value::Duration(dt.as_f64());
            }
            // Serials count days from 1899-12-30, as M does.
            let s = dt.as_f64();
            if s.fract() == 0.0 { Value::Date(s as i64) } else { Value::DateTime(s) }
        }
        Data::DateTimeIso(s) | Data::DurationIso(s) => Value::text(s),
        Data::Error(e) => Value::error(format!("Excel error {e}")),
        Data::Empty => Value::Null,
    }
}

/// A worksheet range as a table: `Column1…` names, or the first row as headers.
pub fn range_table(r: &Range<Data>, use_headers: bool) -> Table {
    let width = r.width();
    let rows: Vec<Vec<Value>> = r.rows().map(|row| row.iter().map(cell).collect()).collect();
    let t = Table::new((1..=width).map(|i| format!("Column{i}")).collect(), rows);
    if use_headers {
        crate::promote_headers(t)
    } else {
        let mut t = t;
        crate::infer_types(&mut t);
        t
    }
}

/// The navigation table of a workbook: `Name`, `Data`, `Item`, `Kind`, `Hidden`.
pub fn workbook(bytes: &[u8], use_headers: bool) -> Result<Table> {
    let mut wb: Sheets<Cursor<Vec<u8>>> = open_workbook_auto_from_rs(Cursor::new(bytes.to_vec()))
        .map_err(|e| Error::Eval(format!("DataFormat.Error: This isn't a workbook we can read ({e}).")))?;
    let mut rows = Vec::new();
    let metas: Vec<(String, bool)> = wb.sheets_metadata().iter().map(|s| (s.name.clone(), s.visible != SheetVisible::Visible)).collect();
    for (name, hidden) in metas {
        let data = match wb.worksheet_range(&name) {
            Ok(r) => Value::table(range_table(&r, use_headers)),
            Err(e) => Value::error(format!("{e}")),
        };
        rows.push(vec![Value::text(&name), data, Value::text(&name), Value::text("Sheet"), Value::Logical(hidden)]);
    }
    if let Sheets::Xlsx(x) = &mut wb
        && x.load_tables().is_ok()
    {
        let names: Vec<String> = x.table_names().into_iter().cloned().collect();
        for n in names {
            if let Ok(tb) = x.table_by_name(&n) {
                let cols: Vec<String> = tb.columns().to_vec();
                let r = tb.data();
                let body: Vec<Vec<Value>> = r.rows().map(|row| row.iter().map(cell).collect()).collect();
                let mut t = Table::new(cols, body);
                crate::infer_types(&mut t);
                rows.push(vec![Value::text(&n), Value::table(t), Value::text(&n), Value::text("Table"), Value::Logical(false)]);
            }
        }
    }
    let mut t = Table::new(["Name", "Data", "Item", "Kind", "Hidden"].iter().map(|s| s.to_string()).collect(), rows);
    t.types = vec![MType::Text, MType::Table, MType::Text, MType::Text, MType::Logical];
    Ok(t)
}
