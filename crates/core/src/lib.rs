//! VizCraft core: everything below the UI.
//!
//! - [`doc`]: the project document (`.vizc`): queries, data sheets, the semantic model, the
//!   report (pages, dashboards, worksheets, stories), bookmarks, parameters, flows, publish targets.
//! - [`build`]: evaluates the Power Query queries and data sheets and builds the in-memory model
//!   (the `vizcraft-datamodel` DAX engine).
//! - [`vquery`]: turns a visual's field wells + filter context into a DAX group-by query.
//! - [`fx`]: dynamic properties: templates with live numbers, conditional rules and gradients.
//! - [`scene`] + [`charts`]: renderer-independent drawing of every visual (painted by egui on the
//!   desktop and as SVG on the web).
//! - [`analytics`]: reference lines, trend lines, forecasting, clustering.
//! - [`tmdl`], [`tableau`], [`sheet`], [`ml`], [`flow`]: TMDL scripts, Tableau-style calculations,
//!   the Excel-like data sheet formula engine, machine learning and Fabric/KNIME-style flows.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod analytics;
pub mod build;
pub mod charts;
pub mod color;
pub mod doc;
pub mod flow;
pub mod format;
pub mod fx;
pub mod ml;
pub mod numfmt;
pub mod sample;
pub mod scene;
pub mod sheet;
pub mod state;
pub mod tableau;
pub mod tmdl;
pub mod vquery;

pub use doc::*;
pub use scalar::Scalar;
pub mod scalar;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Msg(String),
    #[error("DAX: {0}")]
    Dax(#[from] vizcraft_datamodel::Error),
    #[error("Power Query: {0}")]
    Query(String),
}

impl From<vizcraft_query::Error> for Error {
    fn from(e: vizcraft_query::Error) -> Self {
        Error::Query(e.message())
    }
}

pub fn err<T>(m: impl Into<String>) -> Result<T> {
    Err(Error::Msg(m.into()))
}

pub type Result<T> = std::result::Result<T, Error>;
