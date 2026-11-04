//! The engine when VizardCraft is built without the `duckdb` feature: everything reports that SQL
//! isn't available.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use vizardcraft_core::build::TableData;

use crate::{CatalogObject, QueryResult, Result, StatementOutput};

pub const UNAVAILABLE: &str = "This build of VizardCraft has no SQL engine (rebuild with the `duckdb` feature).";

pub struct Engine {
    warehouse: Option<PathBuf>,
    pub attached: BTreeMap<String, String>,
    files: BTreeMap<String, String>,
}

impl Engine {
    pub fn open(warehouse: Option<&Path>) -> Result<Engine> {
        let _ = warehouse;
        Err(UNAVAILABLE.into())
    }
    pub fn warehouse_path(&self) -> Option<&Path> {
        self.warehouse.as_deref()
    }
    pub fn version(&self) -> String {
        String::new()
    }
    pub fn register_table(&mut self, _t: &TableData) -> Result<()> {
        Err(UNAVAILABLE.into())
    }
    pub fn sync_model<'a>(&mut self, _tables: impl IntoIterator<Item = &'a TableData>) -> Vec<(String, String)> {
        vec![]
    }
    pub fn save_table(&self, _name: &str, _t: &TableData) -> Result<()> {
        Err(UNAVAILABLE.into())
    }
    pub fn is_registered(&self, _name: &str) -> bool {
        false
    }
    pub fn model_tables(&self) -> Vec<String> {
        vec![]
    }
    pub fn attach_file(&mut self, _path: &str, _name: Option<&str>) -> Result<String> {
        Err(UNAVAILABLE.into())
    }
    pub fn detach_file(&mut self, _name: &str) -> Result<()> {
        Err(UNAVAILABLE.into())
    }
    pub fn files(&self) -> &BTreeMap<String, String> {
        &self.files
    }
    pub fn load_extension(&self, _name: &str) -> Result<()> {
        Err(UNAVAILABLE.into())
    }
    pub fn attach_database(&mut self, _kind: &str, _target: &str, _alias: &str) -> Result<()> {
        Err(UNAVAILABLE.into())
    }
    pub fn detach_database(&mut self, _alias: &str) -> Result<()> {
        Err(UNAVAILABLE.into())
    }
    pub fn query(&self, _sql: &str, _cap: usize) -> Result<QueryResult> {
        Err(UNAVAILABLE.into())
    }
    pub fn run_script(&self, sql: &str, _cap: usize) -> Vec<StatementOutput> {
        vec![StatementOutput::Error { sql: sql.into(), message: UNAVAILABLE.into() }]
    }
    pub fn explain(&self, _sql: &str, _analyze: bool) -> Result<String> {
        Err(UNAVAILABLE.into())
    }
    pub fn export(&self, _sql: &str, _path: &str) -> Result<()> {
        Err(UNAVAILABLE.into())
    }
    pub fn catalog(&self) -> Result<Vec<CatalogObject>> {
        Ok(vec![])
    }
    pub fn warehouse_tables(&self) -> Vec<String> {
        vec![]
    }
}
