//! The data model as a workbook stores it: which worksheet tables it imports, its relationships,
//! measures, calculated columns and the reports placed on sheets. The columnar store is rebuilt
//! from this definition and the tables' current data ([`DataModelDef::build`]).

use serde::{Deserialize, Serialize};

use crate::model::Model;
use crate::store::{CrossFilter, Table};
use crate::value::Value;
use crate::{Error, Result};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataModelDef {
    #[serde(default)]
    pub tables: Vec<TableDef>,
    #[serde(default)]
    pub relationships: Vec<RelationshipDef>,
    #[serde(default)]
    pub measures: Vec<MeasureDef>,
    #[serde(default)]
    pub calculated_columns: Vec<CalcColumnDef>,
    #[serde(default)]
    pub reports: Vec<ReportDef>,
}

/// A model table, loaded from the worksheet table of the same name, or from a Power Query
/// query when `query` names one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationshipDef {
    pub from_table: String,
    pub from_column: String,
    pub to_table: String,
    pub to_column: String,
    #[serde(default)]
    pub cross_filter: CrossFilter,
    #[serde(default = "yes")]
    pub active: bool,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureDef {
    pub table: String,
    pub name: String,
    pub expression: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalcColumnDef {
    pub table: String,
    pub name: String,
    pub expression: String,
}

/// A data-model PivotTable placed on a sheet: its output is written to cells from `at` and is
/// recomputed by a refresh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportDef {
    pub id: u32,
    pub sheet: String,
    /// Top-left cell (A1 form).
    pub at: String,
    #[serde(default)]
    pub rows: Vec<String>,
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub values: Vec<String>,
    /// DAX filter arguments applied to the whole report.
    #[serde(default)]
    pub filters: Vec<String>,
    /// The size of the block last written (rows, columns), cleared before a refresh writes again.
    #[serde(default)]
    pub extent: (u32, u32),
}

/// The rows of one source table: header names and data rows.
pub struct SourceData {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

impl DataModelDef {
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
            && self.relationships.is_empty()
            && self.measures.is_empty()
            && self.calculated_columns.is_empty()
            && self.reports.is_empty()
    }

    pub fn has_table(&self, name: &str) -> bool {
        self.tables.iter().any(|t| t.name.eq_ignore_ascii_case(name))
    }

    /// Builds the in-memory model. `source` returns a table's current data.
    pub fn build(&self, source: &dyn Fn(&str) -> Option<SourceData>) -> Result<Model> {
        let mut m = Model::new();
        for t in &self.tables {
            let data =
                source(&t.name).ok_or_else(|| Error::Model(format!("the worksheet table `{}` the model loads from no longer exists", t.name)))?;
            m.add_table(Table::from_rows(t.name.clone(), &data.headers, &data.rows)?)?;
        }
        for r in &self.relationships {
            m.add_relationship((&r.from_table, &r.from_column), (&r.to_table, &r.to_column), r.cross_filter, r.active)?;
        }
        // Calculated columns before measures: a column may not refer to a measure added later,
        // but measures may use every column.
        for c in &self.calculated_columns {
            m.add_calculated_column(&c.table, &c.name, &c.expression)?;
        }
        for x in &self.measures {
            m.add_measure(&x.table, &x.name, &x.expression, x.format.clone())?;
        }
        Ok(m)
    }
}
