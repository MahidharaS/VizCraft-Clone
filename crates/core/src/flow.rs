//! Flows: Fabric / KNIME / Orange style data pipelines made of nodes.

use crate::build::TableData;
use crate::doc::Project;
use vizcraft_query::m::eval::Host;

/// The output table of a flow node ("flow::node id").
pub fn table_for(p: &Project, spec: &str, host: &dyn Host) -> crate::Result<TableData> {
    let _ = (p, host);
    crate::err(format!("Flow output `{spec}` is not available."))
}
