//! Filter context: the filters a DAX expression is evaluated under, and how they travel along
//! relationships to give each table its visible rows.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::model::Model;
use crate::store::{CrossFilter, RowSet};
use crate::value::Key;

/// One filter.
#[derive(Clone, Debug)]
pub enum Filter {
    /// Rows of `table` whose `column` holds one of `keys`.
    Values { table: usize, column: usize, keys: Rc<HashSet<Key>> },
    /// Exactly these rows of `table` (a table filter such as `FILTER(Sales, …)`, or a row turned
    /// into a filter by context transition).
    Rows { table: usize, rows: Rc<RowSet> },
}

impl Filter {
    pub fn table(&self) -> usize {
        match self {
            Filter::Values { table, .. } | Filter::Rows { table, .. } => *table,
        }
    }
}

/// The filter context, with a per-context cache of each table's visible rows.
#[derive(Debug, Default)]
pub struct Ctx {
    pub filters: Vec<Filter>,
    /// Relationships turned on by USERELATIONSHIP.
    pub use_rels: Vec<usize>,
    cache: RefCell<HashMap<usize, Rc<RowSet>>>,
}

impl Clone for Ctx {
    fn clone(&self) -> Ctx {
        Ctx { filters: self.filters.clone(), use_rels: self.use_rels.clone(), cache: RefCell::default() }
    }
}

impl Ctx {
    /// Drops every filter on `table`.
    pub fn remove_table(&mut self, table: usize) {
        self.filters.retain(|f| f.table() != table);
    }

    /// Drops the filters on one column (a table filter on its table goes too: it filters every
    /// column of the table).
    pub fn remove_column(&mut self, table: usize, column: usize) {
        self.filters.retain(|f| match f {
            Filter::Values { table: t, column: c, .. } => !(*t == table && *c == column),
            Filter::Rows { table: t, .. } => *t != table,
        });
    }

    /// Adds a filter that overrides the existing ones on the same column (or table).
    pub fn replace(&mut self, f: Filter) {
        match &f {
            Filter::Values { table, column, .. } => self.remove_column(*table, *column),
            Filter::Rows { table, .. } => self.remove_table(*table),
        }
        self.filters.push(f);
    }

    /// Adds a filter that intersects with the existing ones (KEEPFILTERS).
    pub fn keep(&mut self, f: Filter) {
        self.filters.push(f);
    }

    pub fn is_filtered(&self, table: usize, column: Option<usize>) -> bool {
        self.filters.iter().any(|f| match (f, column) {
            (Filter::Values { table: t, column: c, .. }, Some(col)) => *t == table && *c == col,
            (f, _) => f.table() == table,
        })
    }

    fn rel_active(&self, m: &Model, i: usize) -> bool {
        if self.use_rels.contains(&i) {
            return true;
        }
        let Some(r) = m.relationships.get(i) else { return false };
        if !r.active {
            return false;
        }
        // USERELATIONSHIP on another relationship between the same two tables turns this one off.
        !self.use_rels.iter().any(|j| {
            m.relationships.get(*j).is_some_and(|o| {
                (o.from_table == r.from_table && o.to_table == r.to_table) || (o.from_table == r.to_table && o.to_table == r.from_table)
            })
        })
    }

    /// The rows of `table` the context leaves visible.
    pub fn visible(&self, m: &Model, table: usize) -> Rc<RowSet> {
        if let Some(s) = self.cache.borrow().get(&table) {
            return s.clone();
        }
        let n = m.tables.get(table).map(|t| t.row_count()).unwrap_or(0);
        let mut visiting = vec![false; m.tables.len()];
        let set = Rc::new(self.restrict(m, table, &mut visiting).unwrap_or_else(|| RowSet::all(n)));
        self.cache.borrow_mut().insert(table, set.clone());
        set
    }

    /// The rows of `table` its own filters and the filters reaching it through relationships
    /// leave; `None` when nothing filters the table.
    fn restrict(&self, m: &Model, table: usize, visiting: &mut Vec<bool>) -> Option<RowSet> {
        let t = m.tables.get(table)?;
        if let Some(v) = visiting.get_mut(table) {
            *v = true;
        }
        let mut set: Option<RowSet> = None;
        let mut and = |s: RowSet| match set.as_mut() {
            Some(cur) => cur.intersect(&s),
            None => set = Some(s),
        };
        for f in self.filters.iter().filter(|f| f.table() == table) {
            match f {
                Filter::Rows { rows, .. } => and((**rows).clone()),
                Filter::Values { column, keys, .. } => {
                    if let Some(col) = t.column(*column) {
                        let allowed: Vec<bool> = (0..col.cardinality() as u32).map(|id| keys.contains(&col.dict_value(id).key())).collect();
                        and(RowSet::from_rows(
                            t.row_count(),
                            (0..t.row_count())
                                .filter(|r| col.id(*r).is_some_and(|id| allowed.get(id as usize).copied().unwrap_or(false)))
                                .map(|r| r as u32),
                        ));
                    }
                }
            }
        }
        for (i, r) in m.relationships.iter().enumerate() {
            if !self.rel_active(m, i) {
                continue;
            }
            // From the one side down to this (many-side) table.
            if r.from_table == table
                && !visiting.get(r.to_table).copied().unwrap_or(true)
                && let Some(up) = self.restrict(m, r.to_table, visiting)
                && let Some(fk) = t.column(r.from_column)
            {
                let allowed: Vec<bool> =
                    (0..fk.cardinality() as u32).map(|id| r.lookup(&fk.dict_value(id).key()).is_some_and(|row| up.contains(row as usize))).collect();
                and(RowSet::from_rows(
                    t.row_count(),
                    (0..t.row_count())
                        .filter(|row| fk.id(*row).is_some_and(|id| allowed.get(id as usize).copied().unwrap_or(false)))
                        .map(|r| r as u32),
                ));
            }
            // Up from a filtered many side, for bidirectional relationships.
            if r.to_table == table
                && r.cross_filter == CrossFilter::Both
                && !visiting.get(r.from_table).copied().unwrap_or(true)
                && let Some(down) = self.restrict(m, r.from_table, visiting)
                && let (Some(many), Some(pk)) = (m.tables.get(r.from_table).and_then(|ft| ft.column(r.from_column)), t.column(r.to_column))
            {
                let keys: HashSet<Key> = down.iter().map(|row| many.value(row).key()).collect();
                and(RowSet::from_rows(t.row_count(), (0..t.row_count()).filter(|row| keys.contains(&pk.value(*row).key())).map(|r| r as u32)));
            }
        }
        if let Some(v) = visiting.get_mut(table) {
            *v = false;
        }
        set
    }
}
