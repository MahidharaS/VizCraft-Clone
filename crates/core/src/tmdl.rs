//! TMDL (Tabular Model Definition Language) view: the model as an editable script.

use crate::build::Built;
use crate::doc::Project;

/// Writes the model as TMDL.
pub fn write(p: &Project, built: &Built) -> String {
    let _ = built;
    let mut s = format!("model Model\n\tculture: {}\n\n", if p.model.culture.is_empty() { "en-US" } else { &p.model.culture });
    for m in &p.model.measures {
        s += &format!("\tmeasure '{}' = {}\n", m.name, m.expression);
    }
    s
}

/// Applies an edited TMDL script to the project.
pub fn apply(p: &mut Project, script: &str) -> Result<Vec<String>, String> {
    let _ = (p, script);
    Err("TMDL apply is not available yet.".into())
}
