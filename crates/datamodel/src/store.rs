//! The columnar store: dictionary-encoded columns, tables and relationships.
//!
//! Every column keeps each distinct value once (its dictionary) and one `u32` id per row, the
//! layout in-memory analytical engines use: distinct counts, value filters and relationship
//! lookups work on ids instead of values, and a low-cardinality column of a million rows costs
//! four bytes a row.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::value::{Key, Value};
use crate::{Error, Result};

/// The most rows one table may hold (input-derived sizes are capped).
pub const MAX_ROWS: usize = 50_000_000;

/// A set of row numbers of one table, as a bitmap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowSet {
    words: Vec<u64>,
    len: usize,
}

impl RowSet {
    pub fn empty(len: usize) -> RowSet {
        RowSet { words: vec![0; len.div_ceil(64)], len }
    }

    pub fn all(len: usize) -> RowSet {
        let mut s = RowSet { words: vec![u64::MAX; len.div_ceil(64)], len };
        s.trim();
        s
    }

    pub fn from_rows(len: usize, rows: impl IntoIterator<Item = u32>) -> RowSet {
        let mut s = RowSet::empty(len);
        for r in rows {
            s.insert(r as usize);
        }
        s
    }

    fn trim(&mut self) {
        let extra = self.words.len() * 64 - self.len;
        if extra > 0
            && let Some(last) = self.words.last_mut()
        {
            *last &= u64::MAX >> extra;
        }
    }

    pub fn universe(&self) -> usize {
        self.len
    }

    pub fn insert(&mut self, row: usize) {
        if row < self.len
            && let Some(w) = self.words.get_mut(row / 64)
        {
            *w |= 1 << (row % 64);
        }
    }

    pub fn contains(&self, row: usize) -> bool {
        self.words.get(row / 64).is_some_and(|w| w & (1 << (row % 64)) != 0) && row < self.len
    }

    pub fn intersect(&mut self, other: &RowSet) {
        for (i, w) in self.words.iter_mut().enumerate() {
            *w &= other.words.get(i).copied().unwrap_or(0);
        }
    }

    pub fn count(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    pub fn is_full(&self) -> bool {
        self.count() == self.len
    }

    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.words.iter().enumerate().flat_map(|(i, &w)| {
            let mut bits = w;
            std::iter::from_fn(move || {
                if bits == 0 {
                    return None;
                }
                let tz = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                Some(i * 64 + tz)
            })
        })
    }
}

/// The type a column's values have, inferred from its data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataType {
    Empty,
    Number,
    Text,
    Boolean,
    Mixed,
}

/// One dictionary-encoded column.
#[derive(Clone, Debug)]
pub struct Column {
    pub name: String,
    /// Calculated columns keep the DAX that produced them.
    pub expression: Option<String>,
    dict: Vec<Value>,
    index: HashMap<Key, u32>,
    ids: Vec<u32>,
}

impl Column {
    pub fn new(name: impl Into<String>) -> Column {
        Column { name: name.into(), expression: None, dict: Vec::new(), index: HashMap::new(), ids: Vec::new() }
    }

    pub fn from_values(name: impl Into<String>, values: impl IntoIterator<Item = Value>) -> Column {
        let mut c = Column::new(name);
        for v in values {
            c.push(v);
        }
        c
    }

    pub fn push(&mut self, v: Value) {
        let key = v.key();
        let id = match self.index.get(&key) {
            Some(id) => *id,
            None => {
                let id = self.dict.len() as u32;
                self.dict.push(v);
                self.index.insert(key, id);
                id
            }
        };
        self.ids.push(id);
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The value in `row` (blank past the end).
    pub fn value(&self, row: usize) -> &Value {
        static BLANK: Value = Value::Blank;
        self.ids.get(row).and_then(|id| self.dict.get(*id as usize)).unwrap_or(&BLANK)
    }

    pub fn id(&self, row: usize) -> Option<u32> {
        self.ids.get(row).copied()
    }

    /// Number of distinct values (blank included when present).
    pub fn cardinality(&self) -> usize {
        self.dict.len()
    }

    pub fn id_of(&self, key: &Key) -> Option<u32> {
        self.index.get(key).copied()
    }

    pub fn dict_value(&self, id: u32) -> &Value {
        static BLANK: Value = Value::Blank;
        self.dict.get(id as usize).unwrap_or(&BLANK)
    }

    pub fn data_type(&self) -> DataType {
        let mut t = DataType::Empty;
        for v in &self.dict {
            let vt = match v {
                Value::Blank => continue,
                Value::Number(_) => DataType::Number,
                Value::Text(_) => DataType::Text,
                Value::Bool(_) => DataType::Boolean,
            };
            t = match t {
                DataType::Empty => vt,
                same if same == vt => same,
                _ => DataType::Mixed,
            };
        }
        t
    }

    /// Bytes the encoded column takes, roughly (ids plus dictionary).
    pub fn encoded_size(&self) -> usize {
        self.ids.len() * 4
            + self
                .dict
                .iter()
                .map(|v| match v {
                    Value::Text(t) => t.len() + 16,
                    _ => 16,
                })
                .sum::<usize>()
    }
}

/// One table of the model.
#[derive(Clone, Debug)]
pub struct Table {
    pub name: String,
    pub columns: Vec<Column>,
    rows: usize,
}

impl Table {
    /// A table from a header row and data rows (short rows are padded with blanks).
    pub fn from_rows(name: impl Into<String>, headers: &[String], rows: &[Vec<Value>]) -> Result<Table> {
        let name = name.into();
        if rows.len() > MAX_ROWS {
            return Err(Error::Model(format!("table `{name}` has more than {MAX_ROWS} rows")));
        }
        let mut seen = std::collections::HashSet::new();
        for h in headers {
            if h.trim().is_empty() {
                return Err(Error::Model(format!("table `{name}` has a column without a name")));
            }
            if !seen.insert(h.to_lowercase()) {
                return Err(Error::Model(format!("table `{name}` has two columns named `{h}`")));
            }
        }
        let columns = headers
            .iter()
            .enumerate()
            .map(|(ci, h)| Column::from_values(h.clone(), rows.iter().map(|r| r.get(ci).cloned().unwrap_or_default())))
            .collect();
        Ok(Table { name, columns, rows: rows.len() })
    }

    pub fn row_count(&self) -> usize {
        self.rows
    }

    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name.eq_ignore_ascii_case(name))
    }

    pub fn column(&self, i: usize) -> Option<&Column> {
        self.columns.get(i)
    }

    /// Adds (or replaces) a computed column; `values` must have one entry per row.
    pub fn set_column(&mut self, column: Column) -> Result<()> {
        if column.len() != self.rows {
            return Err(Error::Model(format!("column `{}` has {} values for {} rows", column.name, column.len(), self.rows)));
        }
        match self.column_index(&column.name) {
            Some(i) => {
                if let Some(slot) = self.columns.get_mut(i) {
                    *slot = column;
                }
            }
            None => self.columns.push(column),
        }
        Ok(())
    }
}

/// How filters travel along a relationship.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CrossFilter {
    /// From the one side (lookup table) to the many side only.
    #[default]
    Single,
    /// Both ways.
    Both,
}

/// A many-to-one relationship: `from` (many side, the foreign key) → `to` (one side, unique key).
#[derive(Clone, Debug)]
pub struct Relationship {
    pub from_table: usize,
    pub from_column: usize,
    pub to_table: usize,
    pub to_column: usize,
    pub cross_filter: CrossFilter,
    pub active: bool,
    /// One-side key → row, for RELATED and key lookups.
    lookup: HashMap<Key, u32>,
}

impl Relationship {
    pub fn lookup(&self, key: &Key) -> Option<u32> {
        self.lookup.get(key).copied()
    }
}

/// Columns of `to` whose values aren't unique can't be the one side of a relationship.
pub(crate) fn build_relationship(
    tables: &[Table],
    from: (usize, usize),
    to: (usize, usize),
    cross_filter: CrossFilter,
    active: bool,
) -> Result<Relationship> {
    let tt = tables.get(to.0).ok_or_else(|| Error::Model("no such table".into()))?;
    let tc = tt.column(to.1).ok_or_else(|| Error::Model("no such column".into()))?;
    if from.0 == to.0 {
        return Err(Error::Model("a relationship needs two different tables".into()));
    }
    let mut lookup = HashMap::with_capacity(tt.row_count());
    for row in 0..tt.row_count() {
        let v = tc.value(row);
        if v.is_blank() {
            continue;
        }
        if lookup.insert(v.key(), row as u32).is_some() {
            return Err(Error::Model(format!(
                "the column `{}`[{}] has duplicate values (\"{}\"); the lookup side of a relationship needs unique values",
                tt.name,
                tc.name,
                v.to_text()
            )));
        }
    }
    Ok(Relationship { from_table: from.0, from_column: from.1, to_table: to.0, to_column: to.1, cross_filter, active, lookup })
}

/// True when every non-blank value of the column is distinct.
pub(crate) fn is_unique(t: &Table, col: usize) -> bool {
    let Some(c) = t.column(col) else { return false };
    let non_blank = c.dict.iter().filter(|v| !v.is_blank()).count();
    let blank_rows = c.id_of(&Key::Blank).map(|id| c.ids.iter().filter(|x| **x == id).count()).unwrap_or(0);
    non_blank + blank_rows == t.row_count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rowset_ops() {
        let mut a = RowSet::all(130);
        assert_eq!(a.count(), 130);
        assert!(a.contains(129) && !a.contains(130));
        let b = RowSet::from_rows(130, [1, 64, 129, 500]);
        a.intersect(&b);
        assert_eq!(a.iter().collect::<Vec<_>>(), vec![1, 64, 129]);
        assert_eq!(RowSet::all(0).count(), 0);
    }

    #[test]
    fn dictionary_encoding() {
        let c = Column::from_values("Region", ["East", "West", "east", "North"].map(Value::text));
        assert_eq!(c.cardinality(), 3);
        assert_eq!(c.value(2), &Value::text("East"));
        assert_eq!(c.data_type(), DataType::Text);
    }
}
