//! Queries as applied steps.
//!
//! A query is M text. When it is `let A = …, B = … in B`, each binding is an applied step; the
//! editor's actions append a step whose expression starts from the previous one
//! (`Table.RemoveColumns(#"Changed Type", {"X"})`), and step edits rewrite the text. A query that
//! isn't a `let` is one step.

use serde::{Deserialize, Serialize};

use crate::m::lexer::{Tok, lex};
use crate::m::parse_let;
use crate::value::quote_text;
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub name: String,
    pub expr: String,
}

/// Where a query's result goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoadTo {
    /// A table on a worksheet.
    #[default]
    Table,
    /// The data model (Power Pivot), without a worksheet table.
    Model,
    /// Connection only (other queries can use it).
    None,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryDef {
    pub name: String,
    pub m: String,
    #[serde(default)]
    pub load: LoadTo,
    /// The worksheet table the query was loaded to, once loaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loaded_table: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// A parameter: a named value queries refer to by name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Parameter {
    pub name: String,
    /// "Text", "Decimal Number", "Date", "True/False", …
    #[serde(default)]
    pub kind: String,
    /// The value as an M literal (`"East"`, `100`, `#date(2024, 1, 1)`).
    pub value: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Suggested values (as M literals), when the parameter offers a list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
}

/// The queries and parameters a workbook holds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryDoc {
    #[serde(default)]
    pub queries: Vec<QueryDef>,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
}

impl QueryDoc {
    pub fn is_empty(&self) -> bool {
        self.queries.is_empty() && self.parameters.is_empty()
    }

    pub fn query(&self, name: &str) -> Option<&QueryDef> {
        self.queries.iter().find(|q| q.name == name)
    }

    pub fn query_mut(&mut self, name: &str) -> Option<&mut QueryDef> {
        self.queries.iter_mut().find(|q| q.name == name)
    }

    /// A query name not yet used: `base`, `base (2)`, …
    pub fn free_name(&self, base: &str) -> String {
        let taken =
            |n: &str| self.queries.iter().any(|q| q.name.eq_ignore_ascii_case(n)) || self.parameters.iter().any(|p| p.name.eq_ignore_ascii_case(n));
        if !taken(base) {
            return base.to_string();
        }
        (2..).map(|i| format!("{base} ({i})")).find(|n| !taken(n)).unwrap_or_default()
    }

    /// The names of queries and parameters `name` refers to.
    pub fn dependencies(&self, name: &str) -> Vec<String> {
        let Some(q) = self.query(name) else { return vec![] };
        let idents = identifiers(&q.m);
        let mut out: Vec<String> = Vec::new();
        for n in self.queries.iter().map(|q| &q.name).chain(self.parameters.iter().map(|p| &p.name)) {
            if n != name && idents.contains(n) && !out.contains(n) {
                out.push(n.clone());
            }
        }
        out
    }
}

/// `Source` or `#"Changed Type"`.
pub fn ident(name: &str) -> String {
    let plain = name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        && !matches!(
            name,
            "let"
                | "in"
                | "if"
                | "then"
                | "else"
                | "each"
                | "try"
                | "otherwise"
                | "error"
                | "and"
                | "or"
                | "not"
                | "true"
                | "false"
                | "null"
                | "type"
                | "meta"
                | "as"
                | "is"
                | "section"
                | "shared"
        );
    if plain { name.to_string() } else { format!("#{}", quote_text(name)) }
}

/// `[Column]` (or `[#"odd]name"]`): a field reference inside `each`.
pub fn field(name: &str) -> String {
    if name.contains([']', '[', '"']) || name.trim() != name || name.is_empty() { format!("[{}]", ident_quoted(name)) } else { format!("[{name}]") }
}

fn ident_quoted(name: &str) -> String {
    format!("#{}", quote_text(name))
}

/// The steps of a query.
pub fn steps_of(m: &str) -> Result<Vec<Step>> {
    match parse_let(m)? {
        Some(doc) => {
            let mut steps: Vec<Step> =
                doc.bindings.into_iter().map(|(name, span)| Step { name, expr: m.get(span).unwrap_or_default().trim().to_string() }).collect();
            // A result that isn't the last step's name is a step of its own.
            let result = m.get(doc.result).unwrap_or_default().trim().to_string();
            let last = steps.last().map(|s| ident(&s.name));
            if last.as_deref() != Some(result.as_str()) {
                steps.push(Step { name: "Result".into(), expr: result });
            }
            Ok(steps)
        }
        None => Ok(vec![Step { name: "Source".into(), expr: m.trim().to_string() }]),
    }
}

/// M text for steps: `let … in <last step>`.
pub fn build_m(steps: &[Step]) -> String {
    let Some(last) = steps.last() else { return String::new() };
    let mut s = String::from("let\n");
    for (i, st) in steps.iter().enumerate() {
        s.push_str("    ");
        s.push_str(&ident(&st.name));
        s.push_str(" = ");
        s.push_str(&st.expr.replace('\n', "\n    "));
        if i + 1 < steps.len() {
            s.push(',');
        }
        s.push('\n');
    }
    s.push_str("in\n    ");
    s.push_str(&ident(&last.name));
    s
}

/// A step name not used yet: `base`, `base1`, `base2`, …
pub fn free_step_name(steps: &[Step], base: &str) -> String {
    if !steps.iter().any(|s| s.name == base) {
        return base.to_string();
    }
    (1..).map(|i| format!("{base}{i}")).find(|n| !steps.iter().any(|s| s.name == *n)).unwrap_or_default()
}

/// Identifiers used in M text (references to steps, queries, parameters and functions), not
/// counting field names in brackets.
pub fn identifiers(m: &str) -> Vec<String> {
    let Ok(toks) = lex(m) else { return vec![] };
    let mut out = Vec::new();
    let mut prev: Option<&Tok> = None;
    for (t, _) in &toks {
        if let Tok::Ident(n) = t
            && !matches!(prev, Some(Tok::Punct("[")))
        {
            out.push(n.clone());
        }
        prev = Some(t);
    }
    out
}

/// Replaces references to `old` with `new` in M text, leaving field names in brackets alone.
pub fn rename_refs(m: &str, old: &str, new: &str) -> String {
    let Ok(toks) = lex(m) else { return m.to_string() };
    let mut out = String::new();
    let mut last = 0;
    let mut prev: Option<&Tok> = None;
    for (t, start) in &toks {
        if let Tok::Ident(n) = t
            && n == old
            && !matches!(prev, Some(Tok::Punct("[")))
        {
            let len = token_len(m, *start, n);
            out.push_str(m.get(last..*start).unwrap_or_default());
            out.push_str(&ident(new));
            last = start + len;
        }
        prev = Some(t);
    }
    out.push_str(m.get(last..).unwrap_or_default());
    out
}

/// Byte length of the identifier token at `start`.
fn token_len(m: &str, start: usize, name: &str) -> usize {
    let rest = m.get(start..).unwrap_or_default();
    if let Some(body) = rest.strip_prefix("#\"") {
        let mut i = 0;
        let b = body.as_bytes();
        while i < b.len() {
            if b.get(i) == Some(&b'"') {
                if b.get(i + 1) == Some(&b'"') {
                    i += 2;
                    continue;
                }
                return i + 3;
            }
            i += 1;
        }
        rest.len()
    } else {
        name.len()
    }
}

/// Adds a step after `after` (default: the last step). `expr` builds the step's expression from
/// the identifier of the step before it. A step inserted in the middle takes over the next
/// step's input. Returns the new M and the new step's name.
pub fn add_step(m: &str, after: Option<&str>, base: &str, expr: impl FnOnce(&str) -> String) -> Result<(String, String)> {
    let mut steps = steps_of(m)?;
    let idx = match after {
        Some(a) => steps.iter().position(|s| s.name == a).ok_or_else(|| Error::Eval(format!("There is no step named {a}.")))?,
        None => steps.len().checked_sub(1).ok_or_else(|| Error::Eval("The query has no steps.".into()))?,
    };
    let prev = steps.get(idx).map(|s| s.name.clone()).unwrap_or_default();
    let name = free_step_name(&steps, base);
    let e = expr(&ident(&prev));
    if let Some(next) = steps.get_mut(idx + 1) {
        next.expr = rename_refs(&next.expr, &prev, &name);
    }
    steps.insert(idx + 1, Step { name: name.clone(), expr: e });
    Ok((build_m(&steps), name))
}

/// Deletes a step; later steps that used it use the step before it instead.
pub fn delete_step(m: &str, name: &str) -> Result<String> {
    let mut steps = steps_of(m)?;
    let idx = steps.iter().position(|s| s.name == name).ok_or_else(|| Error::Eval(format!("There is no step named {name}.")))?;
    if steps.len() == 1 {
        return Err(Error::Eval("A query needs at least one step.".into()));
    }
    let prev = if idx > 0 { steps.get(idx - 1).map(|s| s.name.clone()) } else { None };
    steps.remove(idx);
    if let Some(p) = prev {
        for s in steps.iter_mut().skip(idx) {
            s.expr = rename_refs(&s.expr, name, &p);
        }
    }
    Ok(build_m(&steps))
}

/// Deletes a step and every step after it.
pub fn delete_until_end(m: &str, name: &str) -> Result<String> {
    let mut steps = steps_of(m)?;
    let idx = steps.iter().position(|s| s.name == name).ok_or_else(|| Error::Eval(format!("There is no step named {name}.")))?;
    if idx == 0 {
        return Err(Error::Eval("A query needs at least one step.".into()));
    }
    steps.truncate(idx);
    Ok(build_m(&steps))
}

pub fn rename_step(m: &str, old: &str, new: &str) -> Result<String> {
    let new = new.trim();
    if new.is_empty() {
        return Err(Error::Eval("A step needs a name.".into()));
    }
    let mut steps = steps_of(m)?;
    if steps.iter().any(|s| s.name == new && s.name != old) {
        return Err(Error::Eval(format!("There is already a step named {new}.")));
    }
    let idx = steps.iter().position(|s| s.name == old).ok_or_else(|| Error::Eval(format!("There is no step named {old}.")))?;
    for s in steps.iter_mut().skip(idx + 1) {
        s.expr = rename_refs(&s.expr, old, new);
    }
    if let Some(s) = steps.get_mut(idx) {
        s.name = new.to_string();
    }
    Ok(build_m(&steps))
}

pub fn set_step_expr(m: &str, name: &str, expr: &str) -> Result<String> {
    crate::m::parse(expr)?;
    let mut steps = steps_of(m)?;
    let s = steps.iter_mut().find(|s| s.name == name).ok_or_else(|| Error::Eval(format!("There is no step named {name}.")))?;
    s.expr = expr.trim().to_string();
    Ok(build_m(&steps))
}

/// Moves a step up (`-1`) or down (`1`) — the editor's Move Before/After.
pub fn move_step(m: &str, name: &str, delta: i32) -> Result<String> {
    let mut steps = steps_of(m)?;
    let i = steps.iter().position(|s| s.name == name).ok_or_else(|| Error::Eval(format!("There is no step named {name}.")))?;
    let j = (i as i64 + delta as i64).clamp(1, steps.len() as i64 - 1) as usize;
    if i == 0 || i == j {
        return Ok(m.to_string());
    }
    // Swap, rewiring the inputs: (prev → a → b) becomes (prev → b → a).
    let (lo, hi) = (i.min(j), i.max(j));
    let prev = steps.get(lo - 1).map(|s| s.name.clone()).unwrap_or_default();
    let a = steps.get(lo).map(|s| s.name.clone()).unwrap_or_default();
    let b = steps.get(hi).map(|s| s.name.clone()).unwrap_or_default();
    if let Some(sb) = steps.get_mut(hi) {
        sb.expr = rename_refs(&sb.expr, &a, &prev);
    }
    if let Some(sa) = steps.get_mut(lo) {
        sa.expr = rename_refs(&sa.expr, &prev, &b);
    }
    for s in steps.iter_mut().skip(hi + 1) {
        s.expr = rename_refs(&s.expr, &b, &a);
    }
    steps.swap(lo, hi);
    Ok(build_m(&steps))
}

/// The query truncated at `step`: what the editor previews when a step is selected.
pub fn m_at_step(m: &str, step: &str) -> Result<String> {
    let steps = steps_of(m)?;
    let i = steps.iter().position(|s| s.name == step).ok_or_else(|| Error::Eval(format!("There is no step named {step}.")))?;
    Ok(build_m(steps.get(..=i).unwrap_or_default()))
}

/// M list literal of texts: `{"A", "B"}`.
pub fn text_list(items: &[String]) -> String {
    format!("{{{}}}", items.iter().map(|s| quote_text(s)).collect::<Vec<_>>().join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const Q: &str = "let\n    Source = #table({\"A\", \"B\"}, {{1, \"x\"}, {2, \"y\"}}),\n    #\"Changed Type\" = Table.TransformColumnTypes(Source, {{\"A\", Int64.Type}}),\n    #\"Removed Columns\" = Table.RemoveColumns(#\"Changed Type\", {\"B\"})\nin\n    #\"Removed Columns\"";

    #[test]
    fn steps_round_trip_and_edit() {
        let s = steps_of(Q).unwrap();
        assert_eq!(s.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), vec!["Source", "Changed Type", "Removed Columns"]);
        assert_eq!(steps_of(&build_m(&s)).unwrap(), s);
        let (m2, name) = add_step(Q, Some("Source"), "Filtered Rows", |p| format!("Table.SelectRows({p}, each [A] > 1)")).unwrap();
        assert_eq!(name, "Filtered Rows");
        let s2 = steps_of(&m2).unwrap();
        assert_eq!(s2[1].name, "Filtered Rows");
        assert!(s2[2].expr.starts_with("Table.TransformColumnTypes(#\"Filtered Rows\""), "{}", s2[2].expr);
        let m3 = delete_step(&m2, "Filtered Rows").unwrap();
        assert_eq!(steps_of(&m3).unwrap(), s);
        let m4 = rename_step(Q, "Changed Type", "Typed").unwrap();
        assert!(m4.contains("Table.RemoveColumns(Typed, {\"B\"})"), "{m4}");
        let m5 = move_step(&m2, "Changed Type", -1).unwrap();
        let s5 = steps_of(&m5).unwrap();
        assert_eq!(s5[1].name, "Changed Type");
        assert!(s5[1].expr.contains("(Source"), "{m5}");
        assert!(s5[2].expr.contains("(#\"Changed Type\""), "{m5}");
        assert_eq!(identifiers("Table.AddColumn(Sales, \"X\", each [Sales] * Rate)").iter().filter(|i| *i == "Sales").count(), 1);
    }
}
