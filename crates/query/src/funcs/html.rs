//! `Web.Page`: the tables of an HTML page.

use crate::value::{MType, Table, Value};

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(rest.get(..i).unwrap_or_default());
        let tail = rest.get(i..).unwrap_or_default();
        let end = tail.find(';').filter(|e| *e <= 10);
        match end {
            Some(e) => {
                let ent = tail.get(1..e).unwrap_or_default();
                let ch = match ent {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" | "#39" => Some('\''),
                    "nbsp" => Some(' '),
                    _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(ent.get(2..).unwrap_or_default(), 16).ok().and_then(char::from_u32),
                    _ if ent.starts_with('#') => ent.get(1..).unwrap_or_default().parse::<u32>().ok().and_then(char::from_u32),
                    _ => None,
                };
                match ch {
                    Some(c) => {
                        out.push(c);
                        rest = tail.get(e + 1..).unwrap_or_default();
                    }
                    None => {
                        out.push('&');
                        rest = tail.get(1..).unwrap_or_default();
                    }
                }
            }
            None => {
                out.push('&');
                rest = tail.get(1..).unwrap_or_default();
            }
        }
    }
    out.push_str(rest);
    out
}

/// Text of an HTML fragment: tags removed, entities decoded, whitespace collapsed.
pub fn text_of(html: &str) -> String {
    let mut s = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                s.push(' ');
            }
            '>' => in_tag = false,
            c if !in_tag => s.push(c),
            _ => {}
        }
    }
    decode_entities(&s).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Case-insensitive find from `from`.
fn find_ci(hay: &str, needle: &str, from: usize) -> Option<usize> {
    let lower = hay.get(from..)?.to_ascii_lowercase();
    lower.find(needle).map(|i| i + from)
}

fn attr(tag: &str, name: &str) -> String {
    let lower = tag.to_ascii_lowercase();
    let Some(i) = lower.find(&format!("{name}=")) else { return String::new() };
    let rest = tag.get(i + name.len() + 1..).unwrap_or_default();
    let q = rest.chars().next().unwrap_or(' ');
    if q == '"' || q == '\'' {
        rest.get(1..).and_then(|r| r.split(q).next()).unwrap_or_default().to_string()
    } else {
        rest.split([' ', '>']).next().unwrap_or_default().to_string()
    }
}

/// One `<table>` as rows of cell texts, with the header row when it uses `<th>`.
fn parse_table(body: &str) -> (Vec<String>, Vec<Vec<String>>) {
    let mut header: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut pos = 0;
    while let Some(tr) = find_ci(body, "<tr", pos) {
        let end = find_ci(body, "</tr", tr).unwrap_or(body.len());
        let row_html = body.get(tr..end).unwrap_or_default();
        let mut cells = Vec::new();
        let mut all_th = true;
        let mut p = 0;
        loop {
            let td = find_ci(row_html, "<td", p);
            let th = find_ci(row_html, "<th", p);
            let (start, is_th) = match (td, th) {
                (Some(a), Some(b)) if b < a => (b, true),
                (Some(a), _) => (a, false),
                (None, Some(b)) => (b, true),
                (None, None) => break,
            };
            let open_end = row_html.get(start..).and_then(|r| r.find('>')).map(|i| start + i + 1).unwrap_or(row_html.len());
            let close = find_ci(row_html, if is_th { "</th" } else { "</td" }, open_end)
                .or_else(|| find_ci(row_html, "<t", open_end))
                .unwrap_or(row_html.len());
            let colspan: usize = attr(row_html.get(start..open_end).unwrap_or_default(), "colspan").parse().unwrap_or(1).clamp(1, 50);
            let txt = text_of(row_html.get(open_end..close).unwrap_or_default());
            for _ in 0..colspan {
                cells.push(txt.clone());
            }
            all_th &= is_th;
            p = close.max(open_end);
        }
        if !cells.is_empty() {
            if all_th && header.is_empty() && rows.is_empty() {
                header = cells;
            } else {
                rows.push(cells);
            }
        }
        pos = end.max(tr + 3);
    }
    (header, rows)
}

/// The tables of a page: `Caption`, `Source`, `ClassName`, `Id`, `Data`.
pub fn web_page(html: &str) -> Table {
    let mut out = Vec::new();
    let mut pos = 0;
    let mut n = 0;
    while let Some(start) = find_ci(html, "<table", pos) {
        // The matching </table>, counting nested tables.
        let mut depth = 0;
        let mut p = start;
        let mut end = html.len();
        while let Some(next) = find_ci(html, "<table", p + 1).into_iter().chain(find_ci(html, "</table", p + 1)).min() {
            if html.get(next..next + 2) == Some("</") {
                if depth == 0 {
                    end = next;
                    break;
                }
                depth -= 1;
            } else {
                depth += 1;
            }
            p = next;
        }
        let tag_end = html.get(start..).and_then(|r| r.find('>')).map(|i| start + i + 1).unwrap_or(start);
        let tag = html.get(start..tag_end).unwrap_or_default();
        let body = html.get(tag_end..end).unwrap_or_default();
        let caption = find_ci(body, "<caption", 0)
            .and_then(|c| {
                let s = body.get(c..)?.find('>')? + c + 1;
                let e = find_ci(body, "</caption", s)?;
                Some(text_of(body.get(s..e)?))
            })
            .unwrap_or_default();
        let (header, rows) = parse_table(body);
        let width = rows.iter().map(Vec::len).max().unwrap_or(0).max(header.len());
        if width > 0 {
            n += 1;
            let mut cols: Vec<String> = (1..=width).map(|i| header.get(i - 1).filter(|h| !h.is_empty()).cloned().unwrap_or_else(|| format!("Column{i}"))).collect();
            // Unique names.
            for i in 0..cols.len() {
                let base = cols.get(i).cloned().unwrap_or_default();
                let mut k = 2;
                while cols.iter().take(i).any(|c| *c == cols.get(i).cloned().unwrap_or_default()) {
                    if let Some(c) = cols.get_mut(i) {
                        *c = format!("{base}{k}");
                    }
                    k += 1;
                }
            }
            let data: Vec<Vec<Value>> = rows
                .into_iter()
                .map(|r| {
                    let mut v: Vec<Value> = r.into_iter().map(Value::text).collect();
                    v.resize(width, Value::Null);
                    v
                })
                .collect();
            let t = Table::new(cols, data);
            out.push(vec![
                Value::text(if caption.is_empty() { format!("Table {n}") } else { caption }),
                Value::text("Table"),
                Value::text(attr(tag, "class")),
                Value::text(attr(tag, "id")),
                Value::table(t),
            ]);
        }
        pos = end.max(start + 6);
    }
    let mut t = Table::new(["Caption", "Source", "ClassName", "Id", "Data"].iter().map(|s| s.to_string()).collect(), out);
    t.types = vec![MType::Text, MType::Text, MType::Text, MType::Text, MType::Table];
    t
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;

    #[test]
    fn tables_from_html() {
        let h = "<html><body><table id=\"t1\"><caption>Prices</caption><tr><th>Item</th><th>Price</th></tr><tr><td>Pen &amp; Ink</td><td>2.50</td></tr><tr><td colspan=2>Total</td></tr></table></body></html>";
        let t = web_page(h);
        assert_eq!(t.rows.len(), 1);
        assert_eq!(t.cell(0, 0).display(), "Prices");
        let Value::Table(d) = t.cell(0, 4) else { panic!("no data") };
        assert_eq!(d.columns, vec!["Item", "Price"]);
        assert_eq!(d.cell(0, 0).display(), "Pen & Ink");
        assert_eq!(d.cell(1, 1).display(), "Total");
    }
}
