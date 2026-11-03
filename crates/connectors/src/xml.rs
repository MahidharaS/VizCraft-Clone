//! `Xml.Tables` and `Xml.Document` (quick-xml).

use quick_xml::Reader;
use quick_xml::events::Event;
use vizcraft_query::{Error, Record, Result, Table, Value};

/// A parsed element.
#[derive(Clone, Debug, Default)]
pub struct Elem {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Elem>,
    pub text: String,
}

pub fn parse(text: &str) -> Result<Elem> {
    let mut r = Reader::from_str(text);
    r.config_mut().trim_text(true);
    let mut stack: Vec<Elem> = vec![Elem { name: "#document".into(), ..Default::default() }];
    let start = |e: &quick_xml::events::BytesStart| -> Elem {
        let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
        let attrs = e
            .attributes()
            .flatten()
            .filter(|a| !a.key.as_ref().starts_with(b"xmlns"))
            .map(|a| (String::from_utf8_lossy(a.key.local_name().as_ref()).to_string(), unescape(&String::from_utf8_lossy(&a.value))))
            .collect();
        Elem { name, attrs, ..Default::default() }
    };
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                if stack.len() > 200 {
                    return Err(Error::Eval("DataFormat.Error: The XML is nested too deeply.".into()));
                }
                stack.push(start(&e));
            }
            Ok(Event::Empty(e)) => {
                let el = start(&e);
                if let Some(p) = stack.last_mut() {
                    p.children.push(el);
                }
            }
            Ok(Event::End(_)) => {
                if stack.len() > 1
                    && let Some(el) = stack.pop()
                    && let Some(p) = stack.last_mut()
                {
                    p.children.push(el);
                }
            }
            Ok(Event::Text(t)) => {
                if let Some(p) = stack.last_mut() {
                    p.text.push_str(&t.decode().map(|s| s.to_string()).unwrap_or_default());
                }
            }
            Ok(Event::CData(t)) => {
                if let Some(p) = stack.last_mut() {
                    p.text.push_str(&String::from_utf8_lossy(&t));
                }
            }
            Ok(Event::GeneralRef(e)) => {
                if let Some(p) = stack.last_mut() {
                    let s = String::from_utf8_lossy(&e).to_string();
                    p.text.push_str(match s.as_str() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => "",
                    });
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::Eval(format!("DataFormat.Error: The XML isn't valid ({e})."))),
            _ => {}
        }
    }
    let doc = stack.into_iter().next().unwrap_or_default();
    doc.children.into_iter().next().ok_or_else(|| Error::Eval("DataFormat.Error: The XML has no root element.".into()))
}

/// An element as a record: its attributes, then its leaf children as text, and repeated or
/// nested children as tables.
fn record(e: &Elem) -> Record {
    let mut names = Vec::new();
    let mut values = Vec::new();
    for (k, v) in &e.attrs {
        names.push(format!("Attribute:{k}"));
        values.push(Value::text(v));
    }
    let mut seen: Vec<String> = Vec::new();
    for c in &e.children {
        if seen.contains(&c.name) {
            continue;
        }
        seen.push(c.name.clone());
        let group: Vec<&Elem> = e.children.iter().filter(|x| x.name == c.name).collect();
        let leaf = group.len() == 1 && c.children.is_empty() && c.attrs.is_empty();
        names.push(c.name.clone());
        values.push(if leaf { Value::text(&c.text) } else { Value::table(group_table(&group)) });
    }
    if e.children.is_empty() && !e.text.is_empty() {
        names.push("Element:Text".into());
        values.push(Value::text(&e.text));
    }
    Record { names, values }
}

fn group_table(els: &[&Elem]) -> Table {
    let recs: Vec<Record> = els.iter().map(|e| record(e)).collect();
    let mut t = crate::table_from_records(&recs);
    crate::infer_types(&mut t);
    t
}

/// `Xml.Tables`: the root's child elements grouped by name, each group as a table.
pub fn tables(text: &str) -> Result<Table> {
    let root = parse(text)?;
    let mut rows = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for c in &root.children {
        if seen.contains(&c.name) {
            continue;
        }
        seen.push(c.name.clone());
        let group: Vec<&Elem> = root.children.iter().filter(|x| x.name == c.name).collect();
        rows.push(vec![Value::text(&c.name), Value::table(group_table(&group))]);
    }
    if rows.is_empty() {
        rows.push(vec![Value::text(&root.name), Value::table(group_table(&[&root]))]);
    }
    Ok(Table::new(vec!["Name".into(), "Table".into()], rows))
}

fn doc_table(els: &[Elem]) -> Table {
    let rows = els
        .iter()
        .map(|e| {
            let attrs = Table::new(
                vec!["Name".into(), "Value".into()],
                e.attrs.iter().map(|(k, v)| vec![Value::text(k), Value::text(v)]).collect(),
            );
            let value = if e.children.is_empty() { Value::text(&e.text) } else { Value::table(doc_table(&e.children)) };
            vec![Value::text(&e.name), Value::text(""), value, Value::table(attrs)]
        })
        .collect();
    Table::new(vec!["Name".into(), "Namespace".into(), "Value".into(), "Attributes".into()], rows)
}

/// `Xml.Document`: the element tree (`Name`, `Namespace`, `Value`, `Attributes`).
pub fn document(text: &str) -> Result<Table> {
    let root = parse(text)?;
    Ok(doc_table(&[root]))
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}
