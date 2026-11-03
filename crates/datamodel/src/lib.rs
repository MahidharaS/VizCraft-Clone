//! GridCraft Power Pivot: a native, in-memory data model engine.
//!
//! - [`store`]: dictionary-encoded columnar tables and many-to-one relationships.
//! - [`dax`]: the DAX language (measures, calculated columns, `EVALUATE` queries).
//! - [`filter`]: filter context and its propagation along relationships.
//! - [`model::Model`]: the model and its query entry points; [`pivot`] lays a query out as a
//!   PivotTable grid; [`def::DataModelDef`] is what a workbook saves.
//!
//! The crate depends on no other GridCraft crate: it can be used on its own.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod dates;
pub mod dax;
pub mod def;
pub mod eval;
pub mod filter;
mod functions;
pub mod model;
pub mod pivot;
pub mod store;
pub mod value;

pub use def::{CalcColumnDef, DataModelDef, MeasureDef, RelationshipDef, ReportDef, SourceData, TableDef};
pub use model::{Model, QueryResult};
pub use store::CrossFilter;
pub use value::Value;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("DAX syntax error: {0}")]
    Parse(String),
    #[error("{0}")]
    Eval(String),
    #[error("{0}")]
    Model(String),
}

pub type Result<T> = std::result::Result<T, Error>;
