//! The data model: tables, relationships and measures, and the entry points that evaluate DAX
//! against it.

use crate::dax::{Expr, parse, parse_query};
use crate::eval::{Env, Evaluator, TableVal};
use crate::filter::Ctx;
use crate::store::{Column, CrossFilter, Relationship, Table, build_relationship, is_unique};
use crate::value::{Value, sort_order};
use crate::{Error, Result};

/// A named DAX formula evaluated in the filter context of the cell that shows it.
#[derive(Clone, Debug)]
pub struct Measure {
    /// The table the measure is stored in (home table); it doesn't limit what it computes.
    pub table: usize,
    pub name: String,
    pub expression: String,
    pub expr: Expr,
    pub format: Option<String>,
}

/// A result table: column names and rows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

impl QueryResult {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "columns": self.columns,
            "rows": self.rows.iter().map(|r| r.iter().map(Value::to_json).collect::<Vec<_>>()).collect::<Vec<_>>(),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub tables: Vec<Table>,
    pub relationships: Vec<Relationship>,
    pub measures: Vec<Measure>,
}

impl Model {
    pub fn new() -> Model {
        Model::default()
    }

    pub fn table_index(&self, name: &str) -> Option<usize> {
        self.tables.iter().position(|t| t.name.eq_ignore_ascii_case(name))
    }

    pub fn table(&self, name: &str) -> Option<&Table> {
        self.table_index(name).and_then(|i| self.tables.get(i))
    }

    pub fn add_table(&mut self, table: Table) -> Result<usize> {
        if self.table_index(&table.name).is_some() {
            return Err(Error::Model(format!("the model already has a table named `{}`", table.name)));
        }
        self.tables.push(table);
        Ok(self.tables.len() - 1)
    }

    /// `(table, column)` indexes of `Table[Column]`.
    pub fn column_ref(&self, table: &str, column: &str) -> Result<(usize, usize)> {
        let ti = self.table_index(table).ok_or_else(|| Error::Model(format!("the model has no table named `{table}`")))?;
        let ci = self
            .tables
            .get(ti)
            .and_then(|t| t.column_index(column))
            .ok_or_else(|| Error::Model(format!("the table `{table}` has no column `{column}`")))?;
        Ok((ti, ci))
    }

    pub fn column_name(&self, t: usize, c: usize) -> String {
        match self.tables.get(t) {
            Some(table) => format!("{}[{}]", quote_table(&table.name), table.column(c).map(|c| c.name.as_str()).unwrap_or("?")),
            None => "?".into(),
        }
    }

    /// Adds a relationship between `from` and `to` columns. Which side is the lookup (one) side
    /// is worked out from the data: the side whose values are unique.
    pub fn add_relationship(&mut self, from: (&str, &str), to: (&str, &str), cross_filter: CrossFilter, active: bool) -> Result<usize> {
        let a = self.column_ref(from.0, from.1)?;
        let b = self.column_ref(to.0, to.1)?;
        let unique = |(t, c): (usize, usize)| self.tables.get(t).is_some_and(|tb| is_unique(tb, c));
        let (many, one) = match (unique(a), unique(b)) {
            (_, true) => (a, b),
            (true, false) => (b, a),
            (false, false) => {
                return Err(Error::Model(format!(
                    "neither {} nor {} has unique values; one side of a relationship must be a lookup column",
                    self.column_name(a.0, a.1),
                    self.column_name(b.0, b.1)
                )));
            }
        };
        if self.relationships.iter().any(|r| (r.from_table, r.from_column, r.to_table, r.to_column) == (many.0, many.1, one.0, one.1)) {
            return Err(Error::Model("that relationship already exists".into()));
        }
        let pair_active = self
            .relationships
            .iter()
            .any(|r| r.active && ((r.from_table, r.to_table) == (many.0, one.0) || (r.from_table, r.to_table) == (one.0, many.0)));
        let rel = build_relationship(&self.tables, many, one, cross_filter, active && !pair_active)?;
        self.relationships.push(rel);
        Ok(self.relationships.len() - 1)
    }

    /// Relationships the data suggests: for two tables not yet related, a column with the same
    /// name in both, unique on one side but not the other, whose values match (most of the many
    /// side's keys are found on the lookup side). Returned as (many side, one side).
    pub fn suggest_relationships(&self) -> Vec<((String, String), (String, String))> {
        let mut out = Vec::new();
        for (ai, a) in self.tables.iter().enumerate() {
            for (bi, b) in self.tables.iter().enumerate().skip(ai + 1) {
                let related = self.relationships.iter().any(|r| (r.from_table == ai && r.to_table == bi) || (r.from_table == bi && r.to_table == ai));
                if related {
                    continue;
                }
                let found = a.columns.iter().enumerate().find_map(|(ac, col)| {
                    let bc = b.column_index(&col.name)?;
                    let (ua, ub) = (is_unique(a, ac), is_unique(b, bc));
                    let (many, one) = match (ua, ub) {
                        (false, true) => ((ai, ac), (bi, bc)),
                        (true, false) => ((bi, bc), (ai, ac)),
                        _ => return None,
                    };
                    self.keys_match(many, one).then_some((many, one))
                });
                if let Some(((mt, mc), (ot, oc))) = found {
                    let name = |t: usize, c: usize| {
                        let tb = self.tables.get(t);
                        (tb.map(|x| x.name.clone()).unwrap_or_default(), tb.and_then(|x| x.column(c)).map(|x| x.name.clone()).unwrap_or_default())
                    };
                    out.push((name(mt, mc), name(ot, oc)));
                }
            }
        }
        out
    }

    /// True when at least half of the many side's distinct non-blank keys exist on the one side.
    fn keys_match(&self, many: (usize, usize), one: (usize, usize)) -> bool {
        let (Some(mc), Some(oc)) = (self.tables.get(many.0).and_then(|t| t.column(many.1)), self.tables.get(one.0).and_then(|t| t.column(one.1)))
        else {
            return false;
        };
        let keys: Vec<_> = (0..mc.cardinality() as u32).map(|id| mc.dict_value(id)).filter(|v| !v.is_blank()).collect();
        if keys.is_empty() {
            return false;
        }
        let hits = keys.iter().filter(|v| oc.id_of(&v.key()).is_some()).count();
        hits * 2 >= keys.len()
    }

    pub fn measure(&self, name: &str) -> Option<&Measure> {
        self.measures.iter().find(|m| m.name.eq_ignore_ascii_case(name))
    }

    pub fn add_measure(&mut self, table: &str, name: &str, expression: &str, format: Option<String>) -> Result<()> {
        let ti = self.table_index(table).ok_or_else(|| Error::Model(format!("the model has no table named `{table}`")))?;
        check_name(name)?;
        if self.measure(name).is_some() {
            return Err(Error::Model(format!("the model already has a measure named `{name}`")));
        }
        if self.tables.iter().any(|t| t.column_index(name).is_some()) {
            return Err(Error::Model(format!("`{name}` is already the name of a column")));
        }
        let expr = parse(expression)?;
        self.measures.push(Measure { table: ti, name: name.to_string(), expression: expression.to_string(), expr, format });
        Ok(())
    }

    /// Adds a calculated column: `expression` is evaluated once per row of `table`, in that row's
    /// context, and stored like imported data.
    pub fn add_calculated_column(&mut self, table: &str, name: &str, expression: &str) -> Result<()> {
        let ti = self.table_index(table).ok_or_else(|| Error::Model(format!("the model has no table named `{table}`")))?;
        check_name(name)?;
        if self.tables.get(ti).and_then(|t| t.column_index(name)).is_some() {
            return Err(Error::Model(format!("the table `{table}` already has a column named `{name}`")));
        }
        if self.measure(name).is_some() {
            return Err(Error::Model(format!("`{name}` is already the name of a measure")));
        }
        let expr = parse(expression)?;
        let rows = self.tables.get(ti).map(|t| t.row_count()).unwrap_or(0);
        let ev = Evaluator::new(self, &[]);
        let ctx = Ctx::default();
        let mut values = Vec::with_capacity(rows);
        for row in 0..rows {
            let env = Env::row(ti, row as u32);
            let v = ev.scalar(&expr, &ctx, &env).map_err(|e| Error::Eval(format!("calculated column `{name}`, row {}: {e}", row + 1)))?;
            values.push(v);
        }
        let mut col = Column::from_values(name, values);
        col.expression = Some(expression.to_string());
        match self.tables.get_mut(ti) {
            Some(t) => t.set_column(col),
            None => Err(Error::Model("no such table".into())),
        }
    }

    /// Evaluates a scalar DAX expression with no filters.
    pub fn evaluate(&self, expression: &str) -> Result<Value> {
        let expr = parse(expression)?;
        Evaluator::new(self, &[]).scalar(&expr, &Ctx::default(), &Env::default())
    }

    /// Evaluates a scalar DAX expression under filters given as DAX filter arguments, as
    /// `CALCULATE(expression, filters…)` would.
    pub fn evaluate_filtered(&self, expression: &str, filters: &[&str]) -> Result<Value> {
        let expr = parse(expression)?;
        let mut args = vec![expr];
        for f in filters {
            args.push(parse(f)?);
        }
        Evaluator::new(self, &[]).scalar(&Expr::Call("CALCULATE".into(), args), &Ctx::default(), &Env::default())
    }

    /// Runs a DAX query (`EVALUATE …`), or a bare table expression.
    pub fn query(&self, text: &str) -> Result<QueryResult> {
        // Leading comments don't count when telling a query from a table expression.
        let mut head = text.trim_start();
        loop {
            if head.starts_with("//") || head.starts_with("--") {
                head = head.split_once('\n').map(|(_, r)| r).unwrap_or("").trim_start();
            } else if let Some(r) = head.strip_prefix("/*") {
                head = r.split_once("*/").map(|(_, r)| r).unwrap_or("").trim_start();
            } else {
                break;
            }
        }
        let q = if head.get(..8).is_some_and(|s| s.eq_ignore_ascii_case("EVALUATE") || s.eq_ignore_ascii_case("DEFINE M"))
            || head.get(..6).is_some_and(|s| s.eq_ignore_ascii_case("DEFINE"))
        {
            parse_query(text)?
        } else {
            crate::dax::Query { measures: vec![], evaluate: parse(text)?, order_by: vec![] }
        };
        let mut local = Vec::new();
        for (t, n, e) in &q.measures {
            self.table_index(t).ok_or_else(|| Error::Model(format!("the model has no table named `{t}`")))?;
            local.push((n.clone(), e.clone()));
        }
        let ev = Evaluator::new(self, &local);
        let ctx = Ctx::default();
        let tv = ev.table(&q.evaluate, &ctx, &Env::default())?;
        let mut res = self.materialize(&tv);
        if !q.order_by.is_empty() {
            let frames: Vec<_> = (0..res.rows.len()).map(|i| crate::eval::frame_of(&tv, i)).collect();
            let mut keyed = Vec::with_capacity(res.rows.len());
            for (row, frame) in res.rows.drain(..).zip(frames) {
                let env = Env { frames: frame.into_iter().collect(), ..Env::default() };
                let mut keys = Vec::new();
                for (e, _) in &q.order_by {
                    keys.push(ev.scalar(e, &ctx, &env)?);
                }
                keyed.push((keys, row));
            }
            keyed.sort_by(|a, b| {
                for (i, (_, desc)) in q.order_by.iter().enumerate() {
                    let o = sort_order(a.0.get(i).unwrap_or(&Value::Blank), b.0.get(i).unwrap_or(&Value::Blank));
                    let o = if *desc { o.reverse() } else { o };
                    if o != std::cmp::Ordering::Equal {
                        return o;
                    }
                }
                std::cmp::Ordering::Equal
            });
            res.rows = keyed.into_iter().map(|(_, r)| r).collect();
        }
        Ok(res)
    }

    /// Group-by query (the shape a PivotTable asks for): one row per existing combination of the
    /// `group_by` columns, with each measure evaluated for it; rows where every measure is blank
    /// are left out. `filters` are DAX filter arguments applied to the whole query.
    pub fn summarize(&self, group_by: &[(String, String)], measures: &[(String, String)], filters: &[String]) -> Result<QueryResult> {
        let mut args = Vec::new();
        for (t, c) in group_by {
            self.column_ref(t, c)?;
            args.push(Expr::Column { table: t.clone(), column: c.clone() });
        }
        for f in filters {
            args.push(Expr::Call("FILTERARG".into(), vec![parse(f)?]));
        }
        for (name, e) in measures {
            args.push(Expr::Text(name.clone()));
            args.push(parse(e)?);
        }
        let ev = Evaluator::new(self, &[]);
        let tv = ev.table(&Expr::Call("SUMMARIZECOLUMNS".into(), args), &Ctx::default(), &Env::default())?;
        Ok(self.materialize(&tv))
    }

    pub(crate) fn materialize(&self, tv: &TableVal) -> QueryResult {
        match tv {
            TableVal::Rows { table, rows } => {
                let Some(t) = self.tables.get(*table) else { return QueryResult::default() };
                QueryResult {
                    columns: t.columns.iter().map(|c| format!("{}[{}]", t.name, c.name)).collect(),
                    rows: rows.iter().map(|r| t.columns.iter().map(|c| c.value(*r as usize).clone()).collect()).collect(),
                }
            }
            TableVal::Values { columns, data } => QueryResult { columns: columns.iter().map(|c| c.display(self)).collect(), rows: data.clone() },
        }
    }

    /// A short description of the model (tables, columns, relationships, measures) as JSON.
    pub fn describe(&self) -> serde_json::Value {
        use serde_json::json;
        json!({
            "tables": self.tables.iter().map(|t| json!({
                "name": t.name,
                "rows": t.row_count(),
                "columns": t.columns.iter().map(|c| json!({
                    "name": c.name,
                    "type": c.data_type(),
                    "distinct": c.cardinality(),
                    "bytes": c.encoded_size(),
                    "calculated": c.expression,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "relationships": self.relationships.iter().map(|r| json!({
                "from": self.column_name(r.from_table, r.from_column),
                "to": self.column_name(r.to_table, r.to_column),
                "crossFilter": r.cross_filter,
                "active": r.active,
            })).collect::<Vec<_>>(),
            "measures": self.measures.iter().map(|m| json!({
                "table": self.tables.get(m.table).map(|t| t.name.clone()),
                "name": m.name,
                "expression": m.expression,
                "format": m.format,
            })).collect::<Vec<_>>(),
        })
    }
}

fn check_name(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.contains(['[', ']']) {
        return Err(Error::Model(format!("`{name}` is not a valid name")));
    }
    Ok(())
}

/// `Sales` or `'Sales Data'`.
pub fn quote_table(name: &str) -> String {
    if name.chars().all(|c| c.is_alphanumeric() || c == '_') && name.chars().next().is_some_and(|c| !c.is_ascii_digit()) {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}
