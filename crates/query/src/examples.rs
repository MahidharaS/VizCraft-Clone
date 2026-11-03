//! Column From Examples: from a few example outputs, find an M formula (an `each` body) that
//! produces them from the row's columns.
//!
//! Candidate formulas are built from the example rows — text as is, case changes, trimming,
//! text before/after/between delimiters, first/last/middle characters, lengths, arithmetic on
//! numbers, rounding, date parts and date formats — and joined with literal text when one piece
//! isn't enough (`Text.Upper(Text.Start([First], 1)) & ". " & [Last]`). Every candidate is run
//! through the M evaluator, so a suggestion is exactly the formula the step will use.

use std::collections::HashSet;

use crate::funcs::convert::{as_text, parse_number};
use crate::m::eval::{Env, Evaluator};
use crate::steps::field;
use crate::value::{MType, Table, Value, quote_text};
use crate::{NoHost, m};

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    /// The formula after `each`.
    pub expr: String,
    /// The type the new column gets.
    pub ty: MType,
}

const DELIMS: &[&str] = &[" ", ",", ", ", "-", " - ", "/", "@", ".", "_", ":", "|", ";", "(", ")", "#", "\\", "=", "+"];
const DATE_FORMATS: &[&str] = &[
    "yyyy-MM",
    "MMM yyyy",
    "MMMM yyyy",
    "yyyy-MM-dd",
    "dd/MM/yyyy",
    "MM/dd/yyyy",
    "dd-MMM-yyyy",
    "yyyyMMdd",
    "MMM-yy",
    "MMM",
    "MMMM",
    "ddd",
    "dddd",
    "dd",
    "yyyy",
];
/// Limits that keep the search fast on wide tables.
const MAX_PIECES: usize = 4;
const MAX_TRIES: usize = 4000;

struct Ctx<'a> {
    ev: Evaluator<'a>,
    rows: Vec<Value>,
    targets: Vec<String>,
}

impl Ctx<'_> {
    /// The formula's output for each example row.
    fn run(&self, expr: &str) -> Option<Vec<Value>> {
        let e = m::parse(&format!("each {expr}")).ok()?;
        let f = self.ev.eval(&e, &Env::default()).ok()?;
        self.rows.iter().map(|r| self.ev.apply1(&f, r.clone()).ok().filter(|v| !v.is_error())).collect()
    }

    fn matches(&self, out: &[Value]) -> bool {
        out.iter().zip(&self.targets).all(|(v, t)| same(v, t))
    }
}

/// Whether a value shows as the example text (numbers compare as numbers).
fn same(v: &Value, t: &str) -> bool {
    match v {
        Value::Number(n) => parse_number(t).is_some_and(|x| (x - n).abs() <= 1e-9 * x.abs().max(1.0)),
        Value::Null => t.is_empty(),
        other => as_text(other).as_deref() == Some(t) || other.display() == t,
    }
}

fn text_of_col(name: &str, is_text: bool) -> String {
    if is_text { field(name) } else { format!("Text.From({})", field(name)) }
}

/// Candidate formulas for one column, with parameters taken from the first example.
fn candidates(name: &str, v: &Value, target: &str, out: &mut Vec<String>) {
    let f = field(name);
    let is_text = matches!(v, Value::Text(_));
    let src = text_of_col(name, is_text);
    let s = as_text(v).unwrap_or_default();
    // Whole values.
    out.push(f.clone());
    for wrap in ["Text.Upper", "Text.Lower", "Text.Proper", "Text.Trim"] {
        out.push(format!("{wrap}({src})"));
    }
    // Delimiters.
    let delims: Vec<&str> = DELIMS.iter().copied().filter(|d| s.contains(d)).collect();
    for d in &delims {
        let n = s.matches(d).count().min(4);
        for i in 0..n {
            out.push(format!("Text.BeforeDelimiter({src}, {}, {i})", quote_text(d)));
            out.push(format!("Text.AfterDelimiter({src}, {}, {i})", quote_text(d)));
        }
        if n > 1 {
            out.push(format!("Text.BeforeDelimiter({src}, {}, {{0, RelativePosition.FromEnd}})", quote_text(d)));
            out.push(format!("Text.AfterDelimiter({src}, {}, {{0, RelativePosition.FromEnd}})", quote_text(d)));
        }
        for d2 in &delims {
            out.push(format!("Text.BetweenDelimiters({src}, {}, {})", quote_text(d), quote_text(d2)));
        }
    }
    // Lengths and positions.
    let tl = target.chars().count();
    let sl = s.chars().count();
    if tl > 0 && tl <= sl {
        out.push(format!("Text.Start({src}, {tl})"));
        out.push(format!("Text.End({src}, {tl})"));
        if let Some(byte) = s.find(target) {
            let start = s.get(..byte).map(|x| x.chars().count()).unwrap_or(0);
            out.push(format!("Text.Middle({src}, {start}, {tl})"));
            out.push(format!("Text.Middle({src}, {start})"));
        }
        for k in 1..=3.min(sl) {
            out.push(format!("Text.Start({src}, {k})"));
        }
    }
    out.push(format!("Text.Length({src})"));
    out.push(format!("Text.Select({src}, {{\"0\"..\"9\"}})"));
    // Numbers.
    if let (Value::Number(x), Some(t)) = (v, parse_number(target)) {
        for d in 0..=3 {
            out.push(format!("Number.Round({f}, {d})"));
        }
        out.push(format!("Number.Abs({f})"));
        out.push(format!("Number.RoundUp({f})"));
        out.push(format!("Number.RoundDown({f})"));
        if *x != 0.0 {
            out.push(format!("{f} * {}", num_lit(t / x)));
            out.push(format!("{f} / {}", num_lit(x / t)));
        }
        out.push(format!("{f} + {}", num_lit(t - x)));
        out.push(format!("{f} - {}", num_lit(x - t)));
        out.push(format!("Number.Power({f}, 2)"));
        out.push(format!("Number.Sqrt({f})"));
    }
    // Dates.
    if matches!(v, Value::Date(_) | Value::DateTime(_)) || (is_text && crate::funcs::convert::parse_date(&s).is_some()) {
        let d = if is_text { format!("Date.From({f})") } else { f.clone() };
        for part in [
            "Date.Year",
            "Date.Month",
            "Date.Day",
            "Date.QuarterOfYear",
            "Date.MonthName",
            "Date.DayOfWeekName",
            "Date.DayOfWeek",
            "Date.WeekOfYear",
            "Date.DayOfYear",
        ] {
            out.push(format!("{part}({d})"));
        }
        out.push(format!("\"Q\" & Text.From(Date.QuarterOfYear({d}))"));
        out.push(format!("Text.From(Date.Year({d})) & \"-Q\" & Text.From(Date.QuarterOfYear({d}))"));
        out.push(format!("Text.Start(Date.MonthName({d}), 3)"));
        out.push(format!("Text.Start(Date.DayOfWeekName({d}), 3)"));
        for fmt in DATE_FORMATS {
            out.push(format!("Date.ToText({d}, \"{fmt}\")"));
        }
    }
}

fn num_lit(x: f64) -> String {
    let r = (x * 1e9).round() / 1e9;
    crate::value::number_text(r)
}

/// Literal-text conditional: `if [c] = "a" then "x" else if … else null`.
fn mapping(t: &Table, inputs: &[usize], examples: &[(usize, String)]) -> Option<String> {
    let [c] = inputs else { return None };
    let name = t.columns.get(*c)?;
    let mut seen = Vec::new();
    let mut parts = Vec::new();
    for (r, out) in examples {
        let v = t.cell(*r, *c).clone();
        if v.is_error() {
            return None;
        }
        match seen.iter().find(|(x, _): &&(Value, String)| *x == v) {
            Some((_, o)) if o != out => return None,
            Some(_) => continue,
            None => {}
        }
        parts.push(format!("if {} = {} then {}", field(name), v.m_literal(), quote_text(out)));
        seen.push((v, out.clone()));
    }
    if parts.len() < 2 {
        return None;
    }
    Some(format!("{} else null", parts.join(" else ")))
}

/// Suggests formulas that produce `examples` (row index, wanted text) from the `inputs` columns
/// (all columns when empty), best first.
pub fn suggest(t: &Table, inputs: &[usize], examples: &[(usize, String)]) -> Vec<Suggestion> {
    let examples: Vec<(usize, String)> = examples.iter().filter(|(r, s)| *r < t.rows.len() && !s.is_empty()).cloned().collect();
    let Some((r0, target0)) = examples.first().cloned() else { return vec![] };
    let inputs: Vec<usize> = if inputs.is_empty() { (0..t.columns.len()).collect() } else { inputs.to_vec() };
    let host = NoHost;
    let cx = Ctx {
        ev: Evaluator::new(&host),
        rows: examples.iter().map(|(r, _)| Value::record(t.row_record(*r))).collect(),
        targets: examples.iter().map(|(_, s)| s.clone()).collect(),
    };
    let numeric = examples.iter().all(|(_, s)| parse_number(s).is_some());
    let ty = if numeric { MType::Number } else { MType::Text };
    let mut pool = Vec::new();
    for c in &inputs {
        if let Some(name) = t.columns.get(*c) {
            candidates(name, t.cell(r0, *c), &target0, &mut pool);
        }
    }
    let mut seen = HashSet::new();
    pool.retain(|e| seen.insert(e.clone()));
    let mut found = Vec::new();
    let mut outputs: Vec<(String, String)> = Vec::new();
    for e in pool.iter().take(MAX_TRIES) {
        let Some(out) = cx.run(e) else { continue };
        if cx.matches(&out) {
            found.push(Suggestion { expr: e.clone(), ty });
            if found.len() >= 5 {
                return found;
            }
        }
        if let Some(first) = out.first().and_then(as_text).filter(|s| !s.is_empty() && target0.contains(s.as_str())) {
            outputs.push((e.clone(), first));
        }
    }
    if found.is_empty() && !numeric {
        found.extend(concatenations(&cx, &target0, &outputs).into_iter().map(|expr| Suggestion { expr, ty: MType::Text }));
    }
    if found.is_empty()
        && let Some(e) = mapping(t, &inputs, &examples)
    {
        found.push(Suggestion { expr: e, ty });
    }
    found
}

/// Pieces of the first example's text, each from a formula or literal, joined with `&`.
fn concatenations(cx: &Ctx, target: &str, outputs: &[(String, String)]) -> Vec<String> {
    let mut results = Vec::new();
    let mut tries = 0usize;
    let mut stack: Vec<(usize, Vec<String>)> = vec![(0, Vec::new())];
    // Longest pieces first: fewer, bigger pieces generalise better.
    let mut sorted: Vec<&(String, String)> = outputs.iter().collect();
    sorted.sort_by_key(|x| std::cmp::Reverse(x.1.len()));
    while let Some((pos, parts)) = stack.pop() {
        if pos >= target.len() {
            if parts.len() < 2 || parts.iter().all(|p| p.starts_with('"')) {
                continue;
            }
            let expr =
                parts
                    .iter()
                    .map(|p| {
                        if p.starts_with('"') || p.starts_with("Text.") || p.starts_with("Date.ToText") {
                            p.clone()
                        } else {
                            format!("Text.From({p})")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" & ");
            tries += 1;
            if let Some(out) = cx.run(&expr)
                && cx.matches(&out)
            {
                results.push(expr);
                if results.len() >= 3 {
                    break;
                }
            }
            if tries > 300 {
                break;
            }
            continue;
        }
        if parts.len() >= MAX_PIECES * 2 {
            continue;
        }
        let rest = target.get(pos..).unwrap_or_default();
        // A literal run up to the next place a formula's output matches.
        let next_match = (1..=rest.len())
            .filter(|i| rest.is_char_boundary(*i))
            .find(|i| sorted.iter().any(|(_, o)| rest.get(*i..).is_some_and(|r| r.starts_with(o.as_str()))))
            .unwrap_or(rest.len());
        if !parts.last().is_some_and(|p| p.starts_with('"')) {
            let lit = rest.get(..next_match).unwrap_or_default();
            if !lit.is_empty() {
                let mut p = parts.clone();
                p.push(quote_text(lit));
                stack.push((pos + next_match, p));
            }
        }
        for (e, o) in sorted.iter().rev() {
            if rest.starts_with(o.as_str()) {
                let mut p = parts.clone();
                p.push(e.clone());
                stack.push((pos + o.len(), p));
            }
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Table {
        let rows = vec![
            vec![Value::text("John Smith"), Value::text("john.smith@contoso.com"), Value::Number(1200.0), Value::Date(45306)],
            vec![Value::text("Mary Jones"), Value::text("mary.jones@fabrikam.org"), Value::Number(80.0), Value::Date(45412)],
            vec![Value::text("Ann Lee"), Value::text("ann.lee@contoso.com"), Value::Number(15.5), Value::Date(45500)],
        ];
        Table::new(vec!["Name".into(), "Email".into(), "Amount".into(), "Order Date".into()], rows)
    }

    fn first(inputs: &[usize], ex: &[(usize, &str)]) -> String {
        let ex: Vec<(usize, String)> = ex.iter().map(|(r, s)| (*r, s.to_string())).collect();
        suggest(&t(), inputs, &ex).first().map(|s| s.expr.clone()).unwrap_or_default()
    }

    #[test]
    fn finds_formulas() {
        assert_eq!(first(&[0], &[(0, "John"), (1, "Mary")]), "Text.BeforeDelimiter([Name], \" \", 0)");
        assert_eq!(first(&[1], &[(0, "contoso.com"), (1, "fabrikam.org")]), "Text.AfterDelimiter([Email], \"@\", 0)");
        assert_eq!(first(&[0], &[(0, "JOHN SMITH")]), "Text.Upper([Name])");
        assert_eq!(first(&[2], &[(0, "2400"), (1, "160")]), "[Amount] * 2");
        assert!(first(&[3], &[(0, "January"), (1, "April")]).contains("Date.MonthName"));
        assert!(first(&[3], &[(0, "2024-01")]).contains("yyyy-MM"));
    }

    #[test]
    fn joins_pieces() {
        let e = first(&[0], &[(0, "Smith, John"), (1, "Jones, Mary")]);
        assert!(e.contains("&"), "{e}");
        let ex = vec![(2usize, "Lee, Ann".to_string())];
        let s = suggest(&t(), &[0], &ex);
        assert!(!s.is_empty());
    }
}
