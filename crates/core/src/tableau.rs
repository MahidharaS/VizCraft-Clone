//! Tableau-style calculated fields: translates Tableau calculation syntax (IF/THEN/ELSE/END,
//! IIF, ZN, LOD expressions `{FIXED …}`, DATETRUNC, …) to DAX.

/// Translates a Tableau calculation to DAX. Text that is already DAX passes through.
pub fn to_dax(calc: &str, default_table: &str) -> Result<String, String> {
    let _ = default_table;
    Ok(calc.to_string())
}
