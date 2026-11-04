//! Field signatures: what kind of field the user put somewhere (date, measure, category, geo,
//! ID…) — the features the learners see. Names are kept only for the question memory; the
//! learners generalise across projects through the types.

use serde::{Deserialize, Serialize};
use vizcraft_core::build::{Built, ColType};
use vizcraft_core::doc::{FieldRef, FieldUse, Project};

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FieldSig {
    /// "date", "measure", "numeric", "category", "geo", "id", "bool".
    pub ty: String,
    pub name: String,
    /// Cardinality bucket for categories: "low" (≤ 7), "mid" (≤ 30), "high".
    pub card: String,
}

impl FieldSig {
    pub fn new(ty: &str, name: &str) -> FieldSig {
        FieldSig { ty: ty.into(), name: name.into(), card: String::new() }
    }
    pub fn is_value(&self) -> bool {
        matches!(self.ty.as_str(), "measure" | "numeric")
    }
}

const GEO: &[&str] = &["country", "region", "state", "city", "province", "county", "zip", "postal", "postcode", "latitude", "longitude", "lat", "lon", "address", "territory"];

/// A geographic-sounding name only counts when most sampled values are places the map knows
/// ("Region" holding East/West is a category, not a location). Coordinates always count.
fn places_resolve(built: &Built, table: &str, column: &str) -> bool {
    let l = column.to_lowercase();
    if ["latitude", "longitude", "lat", "lon"].iter().any(|g| l == *g || l.ends_with(&format!(" {g}"))) {
        return true;
    }
    let Some(t) = built.table(table) else { return false };
    let Some(i) = t.column_index(column) else { return false };
    let vals: std::collections::BTreeSet<String> = t.rows.iter().take(5_000).filter_map(|r| r.get(i).map(|v| v.text())).filter(|v| !v.is_empty()).take(200).collect();
    let hits = vals.iter().filter(|v| vizcraft_core::charts::world::lookup(v).is_some()).count();
    !vals.is_empty() && hits * 2 > vals.len()
}

/// Classifies a column by name, type and values.
pub fn classify_column(p: &Project, built: &Built, table: &str, column: &str) -> FieldSig {
    let lname = column.to_lowercase();
    let ty = built.col_type(table, column);
    let category = p.model.tables.iter().find(|t| t.name == table).and_then(|t| t.columns.iter().find(|c| c.name == column)).map(|c| c.data_category.to_lowercase()).unwrap_or_default();
    let distinct = built.table(table).and_then(|t| {
        let i = t.column_index(column)?;
        let set: std::collections::HashSet<String> = t.rows.iter().take(20_000).map(|r| r.get(i).map(|v| v.key()).unwrap_or_default()).collect();
        Some((set.len(), t.len()))
    });
    let is_id = lname == "id" || lname.ends_with(" id") || lname.ends_with("_id") || lname.ends_with("id") && column.chars().rev().nth(2).is_some_and(char::is_lowercase) || lname.ends_with("key") || lname.ends_with("code") && ty != ColType::Text;
    let t = if ty.is_temporal() || lname == "year" || lname == "month" || lname == "quarter" || lname.ends_with(".year") || lname.ends_with(".month") {
        "date"
    } else if !category.is_empty() && category != "uncategorized" || GEO.iter().any(|g| lname == *g || lname.ends_with(&format!(" {g}"))) && places_resolve(built, table, column) {
        "geo"
    } else if is_id && distinct.is_some_and(|(d, n)| n > 0 && d * 10 >= n * 9) {
        "id"
    } else if ty == ColType::Bool {
        "bool"
    } else if ty.is_numeric() && !is_id {
        "numeric"
    } else {
        "category"
    };
    let card = match distinct {
        Some((d, _)) if t == "category" || t == "geo" => {
            if d <= 7 {
                "low"
            } else if d <= 30 {
                "mid"
            } else {
                "high"
            }
        }
        _ => "",
    };
    FieldSig { ty: t.into(), name: column.to_string(), card: card.into() }
}

pub fn classify(p: &Project, built: &Built, f: &FieldRef) -> FieldSig {
    match f {
        FieldRef::Measure { name, .. } => FieldSig::new("measure", name),
        FieldRef::Column { table, column } => classify_column(p, built, table, column),
        FieldRef::Level { level, .. } => FieldSig::new("date", level),
        FieldRef::FieldParam { name } => FieldSig::new("category", name),
    }
}

/// Signature of a field use: an aggregated column counts as a value.
pub fn classify_use(p: &Project, built: &Built, fu: &FieldUse) -> FieldSig {
    let mut s = classify(p, built, &fu.field);
    if fu.is_value() && !s.is_value() {
        s.ty = "numeric".into();
        s.card.clear();
    }
    s
}

/// The context key the count model uses: sorted field types ("category+date+measure").
pub fn context_key(fields: &[FieldSig]) -> String {
    let mut types: Vec<&str> = fields.iter().map(|f| if f.ty == "numeric" { "measure" } else { f.ty.as_str() }).collect();
    types.sort_unstable();
    types.dedup();
    if types.is_empty() { "empty".into() } else { types.join("+") }
}

/// Human phrase for a context ("a date and a measure").
pub fn context_phrase(fields: &[FieldSig]) -> String {
    let count = |t: &[&str]| fields.iter().filter(|f| t.contains(&f.ty.as_str())).count();
    let mut parts = Vec::new();
    let d = count(&["date"]);
    let m = count(&["measure", "numeric"]);
    let c = count(&["category", "bool", "id"]);
    let g = count(&["geo"]);
    let word = |n: usize, one: &str, many: &str| if n == 1 { format!("a {one}") } else { format!("{n} {many}") };
    if d > 0 {
        parts.push(word(d, "date", "dates"));
    }
    if g > 0 {
        parts.push(word(g, "location", "locations"));
    }
    if c > 0 {
        parts.push(word(c, "category", "categories"));
    }
    if m > 0 {
        parts.push(word(m, "measure", "measures"));
    }
    match parts.len() {
        0 => "no fields".into(),
        1 => parts.remove(0),
        _ => {
            let last = parts.pop().unwrap_or_default();
            format!("{} and {last}", parts.join(", "))
        }
    }
}

/// Feature vector for the logistic recommender (fixed length [`N_FEATURES`]).
pub const FEATURE_NAMES: &[&str] = &[
    "bias", "has_date", "measures=1", "measures=2", "measures>=3", "cats=0", "cats=1", "cats>=2", "has_geo", "has_id", "card_low", "card_mid", "card_high", "only_values", "has_bool",
];
pub const N_FEATURES: usize = 15;

pub fn features(fields: &[FieldSig]) -> [f64; N_FEATURES] {
    let n = |t: &[&str]| fields.iter().filter(|f| t.contains(&f.ty.as_str())).count();
    let m = n(&["measure", "numeric"]);
    let c = n(&["category", "bool", "id"]);
    let card = |b: &str| fields.iter().any(|f| f.card == b);
    let b = |x: bool| if x { 1.0 } else { 0.0 };
    [
        1.0,
        b(n(&["date"]) > 0),
        b(m == 1),
        b(m == 2),
        b(m >= 3),
        b(c == 0 && n(&["date", "geo"]) == 0),
        b(c == 1),
        b(c >= 2),
        b(n(&["geo"]) > 0),
        b(n(&["id"]) > 0),
        b(card("low")),
        b(card("mid")),
        b(card("high")),
        b(m > 0 && c + n(&["date", "geo"]) == 0),
        b(n(&["bool"]) > 0),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use vizcraft_core::build;

    #[test]
    fn classifies_sample_fields() {
        let mut p = vizcraft_core::sample::sales();
        build::sync_model_tables(&mut p);
        let b = build::build(&p, &vizcraft_connectors::DesktopHost::default(), &Default::default());
        assert_eq!(classify(&p, &b, &FieldRef::col("Sales", "Date")).ty, "date");
        assert_eq!(classify(&p, &b, &FieldRef::measure("Sales", "Total Sales")).ty, "measure");
        // The sample's Region is East/North/South/West: not places the map can plot.
        let region = classify(&p, &b, &FieldRef::col("Store", "Region"));
        assert_eq!(region.ty, "category");
        assert_eq!(context_key(&[FieldSig::new("measure", "a"), FieldSig::new("date", "b")]), "date+measure");
        assert_eq!(context_phrase(&[FieldSig::new("measure", "a"), FieldSig::new("date", "b")]), "a date and a measure");
    }
}
