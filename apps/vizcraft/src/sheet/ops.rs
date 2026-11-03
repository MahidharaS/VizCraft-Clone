//! Data operations on a sheet's raw cells: set, paste, fill, insert / delete rows and columns,
//! sort, CSV import and export.

use vizcraft_core::doc::DataSheet;
use vizcraft_core::scalar::Scalar;
use vizcraft_core::sheet::{self, CellValue};

/// A rectangle of cells (inclusive).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub r1: usize,
    pub c1: usize,
    pub r2: usize,
    pub c2: usize,
}

impl Range {
    pub fn new(a: (usize, usize), b: (usize, usize)) -> Range {
        Range { r1: a.0.min(b.0), c1: a.1.min(b.1), r2: a.0.max(b.0), c2: a.1.max(b.1) }
    }
    pub fn contains(&self, r: usize, c: usize) -> bool {
        r >= self.r1 && r <= self.r2 && c >= self.c1 && c <= self.c2
    }
    pub fn rows(&self) -> usize {
        self.r2 - self.r1 + 1
    }
    pub fn cols(&self) -> usize {
        self.c2 - self.c1 + 1
    }
    pub fn label(&self) -> String {
        if self.r1 == self.r2 && self.c1 == self.c2 {
            sheet::address(self.r1, self.c1)
        } else {
            format!("{}:{}", sheet::address(self.r1, self.c1), sheet::address(self.r2, self.c2))
        }
    }
}

pub fn get(s: &DataSheet, r: usize, c: usize) -> &str {
    s.cells.get(r).and_then(|row| row.get(c)).map(String::as_str).unwrap_or("")
}

pub fn width(s: &DataSheet) -> usize {
    s.cells.iter().map(|r| r.iter().rposition(|c| !c.is_empty()).map(|i| i + 1).unwrap_or(0)).max().unwrap_or(0).max(s.cells.first().map(|r| r.len()).unwrap_or(0))
}

pub fn set(s: &mut DataSheet, r: usize, c: usize, v: &str) {
    if v.is_empty() && get(s, r, c).is_empty() {
        return;
    }
    while s.cells.len() <= r {
        s.cells.push(vec![]);
    }
    if let Some(row) = s.cells.get_mut(r) {
        if row.len() <= c {
            row.resize(c + 1, String::new());
        }
        if let Some(cell) = row.get_mut(c) {
            *cell = v.to_string();
        }
    }
}

/// Removes trailing empty rows and trailing empty cells (keeps the header row's width).
pub fn trim(s: &mut DataSheet) {
    while s.cells.len() > 1 && s.cells.last().is_some_and(|r| r.iter().all(|c| c.is_empty())) {
        s.cells.pop();
    }
    let hw = s.cells.first().map(|r| r.len()).unwrap_or(0);
    for (i, row) in s.cells.iter_mut().enumerate() {
        let keep = row.iter().rposition(|c| !c.is_empty()).map(|p| p + 1).unwrap_or(0);
        let keep = if i == 0 { keep.max(hw.min(row.len())) } else { keep };
        row.truncate(keep);
    }
}

/// Parses clipboard text (TSV from Excel / Google Sheets; quoted cells allowed).
pub fn parse_tsv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    let text = text.strip_suffix('\r').unwrap_or(text);
    if text.contains('"') {
        return vizcraft_core::flow::parse_csv(&format!("{text}\n"), '\t');
    }
    text.split('\n').map(|l| l.trim_end_matches('\r').split('\t').map(str::to_string).collect()).collect()
}

/// TSV of displayed values of a range.
pub fn copy_values(grid: &[Vec<CellValue>], s: &DataSheet, r: Range) -> String {
    let mut out = String::new();
    for row in r.r1..=r.r2 {
        let cells: Vec<String> = (r.c1..=r.c2)
            .map(|c| {
                let v = grid.get(row).and_then(|x| x.get(c)).cloned().unwrap_or_default();
                let f = s.col_formats.get(c).map(String::as_str).unwrap_or("");
                let t = match &v {
                    CellValue::Num(_) if !f.is_empty() && vizcraft_core::numfmt::is_date_format(f) => v.display(f),
                    CellValue::Num(n) => vizcraft_core::numfmt::general(*n),
                    other => other.display(""),
                };
                if t.contains('\t') || t.contains('\n') || t.contains('"') { format!("\"{}\"", t.replace('"', "\"\"")) } else { t }
            })
            .collect();
        out.push_str(&cells.join("\t"));
        out.push('\n');
    }
    out
}

/// Raw inputs of a range.
pub fn copy_raw(s: &DataSheet, r: Range) -> Vec<Vec<String>> {
    (r.r1..=r.r2).map(|row| (r.c1..=r.c2).map(|c| get(s, row, c).to_string()).collect()).collect()
}

/// Pastes a block at (r, c); a single cell fills the whole `target` range. Formulas copied from
/// `origin` are shifted. Returns the pasted range.
pub fn paste(s: &mut DataSheet, block: &[Vec<String>], at: (usize, usize), target: Range, origin: Option<(usize, usize)>) -> Range {
    let bh = block.len().max(1);
    let bw = block.iter().map(Vec::len).max().unwrap_or(1).max(1);
    // Repeat the block over a larger selection when it fits a whole number of times.
    let (h, w) = if target.rows() % bh == 0 && target.cols() % bw == 0 && (target.rows() > bh || target.cols() > bw) { (target.rows(), target.cols()) } else { (bh, bw) };
    let (r0, c0) = if h == target.rows() && w == target.cols() && (h > bh || w > bw) { (target.r1, target.c1) } else { at };
    for i in 0..h {
        for j in 0..w {
            let src = block.get(i % bh).and_then(|row| row.get(j % bw)).cloned().unwrap_or_default();
            let v = match origin {
                Some((orow, ocol)) if src.starts_with('=') => {
                    let from_r = orow + i % bh;
                    let from_c = ocol + j % bw;
                    sheet::shift_formula(&src, (r0 + i) as i64 - from_r as i64, (c0 + j) as i64 - from_c as i64)
                }
                _ => src,
            };
            set(s, r0 + i, c0 + j, &v);
        }
    }
    Range { r1: r0, c1: c0, r2: r0 + h - 1, c2: c0 + w - 1 }
}

/// Fills a range from its first row (down) — Ctrl+D.
pub fn fill_down(s: &mut DataSheet, r: Range) {
    for c in r.c1..=r.c2 {
        let src = get(s, r.r1, c).to_string();
        for row in (r.r1 + 1)..=r.r2 {
            let v = if src.starts_with('=') { sheet::shift_formula(&src, (row - r.r1) as i64, 0) } else { src.clone() };
            set(s, row, c, &v);
        }
    }
}

/// Fill handle: extends `src` to cover `dst` (which contains `src`), continuing number series,
/// dates and "Item 1" style text; formulas shift.
pub fn fill_series(s: &mut DataSheet, src: Range, dst: Range) {
    let vertical = dst.rows() != src.rows();
    if vertical {
        for c in src.c1..=src.c2 {
            let seq: Vec<String> = (src.r1..=src.r2).map(|r| get(s, r, c).to_string()).collect();
            let down = dst.r2 > src.r2;
            let targets: Vec<usize> = if down { ((src.r2 + 1)..=dst.r2).collect() } else { (dst.r1..src.r1).rev().collect() };
            for (k, row) in targets.iter().enumerate() {
                let v = continue_series(&seq, k + 1, down, *row as i64 - if down { src.r2 as i64 } else { src.r1 as i64 }, true);
                set(s, *row, c, &v);
            }
        }
    } else {
        for r in src.r1..=src.r2 {
            let seq: Vec<String> = (src.c1..=src.c2).map(|c| get(s, r, c).to_string()).collect();
            let right = dst.c2 > src.c2;
            let targets: Vec<usize> = if right { ((src.c2 + 1)..=dst.c2).collect() } else { (dst.c1..src.c1).rev().collect() };
            for (k, col) in targets.iter().enumerate() {
                let v = continue_series(&seq, k + 1, right, *col as i64 - if right { src.c2 as i64 } else { src.c1 as i64 }, false);
                set(s, r, *col, &v);
            }
        }
    }
}

/// The k-th value after (forward) or before a sequence.
fn continue_series(seq: &[String], k: usize, forward: bool, offset: i64, vertical: bool) -> String {
    let n = seq.len();
    if n == 0 {
        return String::new();
    }
    // Formulas and plain text repeat the pattern (formulas shifted).
    let pick = if forward { (k - 1) % n } else { n - 1 - (k - 1) % n };
    let base = seq.get(pick).cloned().unwrap_or_default();
    if base.starts_with('=') {
        // Shift relative to where the source cell is.
        let src_pos = if forward { (n - 1 - pick) as i64 } else { -(pick as i64) };
        let d = offset + src_pos;
        return if vertical { sheet::shift_formula(&base, d, 0) } else { sheet::shift_formula(&base, 0, d) };
    }
    let nums: Vec<Option<f64>> = seq.iter().map(|v| match sheet::parse_input(v) {
        Scalar::Num(x) => Some(x),
        _ => None,
    }).collect();
    if nums.iter().all(Option::is_some) {
        let xs: Vec<f64> = nums.into_iter().flatten().collect();
        let step = if xs.len() >= 2 {
            // Least-squares slope (Excel's linear trend for numbers).
            let m = xs.len() as f64;
            let mx = (m - 1.0) / 2.0;
            let my = xs.iter().sum::<f64>() / m;
            let num: f64 = xs.iter().enumerate().map(|(i, y)| (i as f64 - mx) * (y - my)).sum();
            let den: f64 = (0..xs.len()).map(|i| (i as f64 - mx).powi(2)).sum();
            if den > 0.0 { num / den } else { 0.0 }
        } else {
            0.0
        };
        if xs.len() >= 2 {
            let last = if forward { xs.last().copied().unwrap_or(0.0) } else { xs.first().copied().unwrap_or(0.0) };
            let v = if forward { last + step * k as f64 } else { last - step * k as f64 };
            return vizcraft_core::numfmt::general(v);
        }
        return base;
    }
    // Dates step a day; "Item 7" steps its trailing number.
    if n == 1 {
        let b = base.trim();
        if b.len() >= 8 && b.chars().next().is_some_and(|c| c.is_ascii_digit()) && let Some(d) = vizcraft_core::build::parse_date(b) {
            let v = if forward { d + k as f64 } else { d - k as f64 };
            return vizcraft_core::numfmt::format_date(v, "yyyy-mm-dd");
        }
        let digits = b.len() - b.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        if digits > 0 && digits < b.len() {
            let (head, num) = b.split_at(b.len() - digits);
            if let Ok(x) = num.parse::<i64>() {
                let v = if forward { x + k as i64 } else { (x - k as i64).max(0) };
                return format!("{head}{v}");
            }
        }
    }
    base
}

pub fn insert_rows(s: &mut DataSheet, at: usize, n: usize) {
    for row in s.cells.iter_mut() {
        for c in row.iter_mut() {
            if c.starts_with('=') {
                *c = sheet::adjust_refs(c, true, at, n as i64);
            }
        }
    }
    let at = at.min(s.cells.len());
    for _ in 0..n {
        s.cells.insert(at, vec![]);
    }
}

pub fn delete_rows(s: &mut DataSheet, at: usize, n: usize) {
    let end = (at + n).min(s.cells.len());
    if at < end {
        s.cells.drain(at..end);
    }
    for row in s.cells.iter_mut() {
        for c in row.iter_mut() {
            if c.starts_with('=') {
                *c = sheet::adjust_refs(c, true, at, -(n as i64));
            }
        }
    }
}

fn shift_vec<T: Clone + Default>(v: &mut Vec<T>, at: usize, n: usize, insert: bool) {
    if insert {
        if at <= v.len() {
            for _ in 0..n {
                v.insert(at, T::default());
            }
        }
    } else {
        let end = (at + n).min(v.len());
        if at < end {
            v.drain(at..end);
        }
    }
}

pub fn insert_cols(s: &mut DataSheet, at: usize, n: usize) {
    for row in s.cells.iter_mut() {
        for c in row.iter_mut() {
            if c.starts_with('=') {
                *c = sheet::adjust_refs(c, false, at, n as i64);
            }
        }
        if at <= row.len() {
            shift_vec(row, at, n, true);
        }
    }
    shift_vec(&mut s.col_widths, at, n, true);
    shift_vec(&mut s.col_types, at, n, true);
    shift_vec(&mut s.col_formats, at, n, true);
    for w in s.col_widths.iter_mut() {
        if *w <= 0.0 {
            *w = 0.0;
        }
    }
}

pub fn delete_cols(s: &mut DataSheet, at: usize, n: usize) {
    for row in s.cells.iter_mut() {
        shift_vec(row, at, n, false);
        for c in row.iter_mut() {
            if c.starts_with('=') {
                *c = sheet::adjust_refs(c, false, at, -(n as i64));
            }
        }
    }
    shift_vec(&mut s.col_widths, at, n, false);
    shift_vec(&mut s.col_types, at, n, false);
    shift_vec(&mut s.col_formats, at, n, false);
}

/// Sorts the data rows (below the header) by a column's evaluated values.
pub fn sort_rows(s: &mut DataSheet, grid: &[Vec<CellValue>], col: usize, desc: bool) {
    if s.cells.len() <= 2 {
        return;
    }
    let mut idx: Vec<usize> = (1..s.cells.len()).collect();
    let key = |r: usize| grid.get(r).and_then(|row| row.get(col)).cloned().unwrap_or_default();
    idx.sort_by(|a, b| {
        let (x, y) = (key(*a), key(*b));
        // Empty cells always last.
        match (matches!(x, CellValue::Empty), matches!(y, CellValue::Empty)) {
            (true, true) => return std::cmp::Ordering::Equal,
            (true, false) => return std::cmp::Ordering::Greater,
            (false, true) => return std::cmp::Ordering::Less,
            _ => {}
        }
        let o = vizcraft_core::scalar::compare(&x.to_scalar(), &y.to_scalar());
        if desc { o.reverse() } else { o }
    });
    let mut rows = vec![s.cells.first().cloned().unwrap_or_default()];
    rows.extend(idx.iter().filter_map(|i| s.cells.get(*i).cloned()));
    s.cells = rows;
}

/// A sheet from CSV text.
pub fn sheet_from_csv(name: &str, text: &str) -> DataSheet {
    let delim = vizcraft_core::flow::detect_delimiter(text);
    let rows = vizcraft_core::flow::parse_csv(text, delim);
    let t = vizcraft_core::flow::table_from_rows(name, rows.clone(), true);
    let mut cells: Vec<Vec<String>> = vec![t.columns.clone()];
    cells.extend(rows.into_iter().skip(1));
    let col_types = t.types.iter().map(|ty| match ty {
        vizcraft_core::build::ColType::Date => "Date".to_string(),
        _ => String::new(),
    }).collect();
    DataSheet { name: name.into(), cells, col_types, load: true, ..DataSheet::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(rows: &[&[&str]]) -> DataSheet {
        DataSheet { name: "S".into(), cells: rows.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect(), ..DataSheet::default() }
    }

    #[test]
    fn fill_and_series() {
        let mut s = sh(&[&["a", "b"], &["1", "=A2*2"], &["2", ""]]);
        fill_series(&mut s, Range::new((1, 0), (2, 0)), Range::new((1, 0), (5, 0)));
        assert_eq!((3..6).map(|r| get(&s, r, 0).to_string()).collect::<Vec<_>>(), vec!["3", "4", "5"]);
        fill_down(&mut s, Range::new((1, 1), (3, 1)));
        assert_eq!(get(&s, 3, 1), "=A4*2");
        let mut s2 = sh(&[&["d"], &["2024-01-30"], &["Item 9"]]);
        fill_series(&mut s2, Range::new((1, 0), (1, 0)), Range::new((1, 0), (2, 0)));
        assert_eq!(get(&s2, 2, 0), "2024-01-31");
    }

    #[test]
    fn insert_delete_and_paste() {
        let mut s = sh(&[&["a", "b"], &["1", "=A2+1"], &["2", "=SUM(A2:A3)"]]);
        insert_rows(&mut s, 2, 1);
        assert_eq!(get(&s, 3, 1), "=SUM(A2:A4)");
        delete_rows(&mut s, 2, 1);
        assert_eq!(get(&s, 2, 1), "=SUM(A2:A3)");
        insert_cols(&mut s, 0, 1);
        assert_eq!(get(&s, 1, 2), "=B2+1");
        let block = parse_tsv("x\ty\r\n1\t2\r\n");
        assert_eq!(block, vec![vec!["x", "y"], vec!["1", "2"]]);
        let r = paste(&mut s, &[vec!["=A1".to_string()]], (5, 0), Range::new((5, 0), (6, 1)), Some((0, 0)));
        assert_eq!(r, Range { r1: 5, c1: 0, r2: 6, c2: 1 });
        assert_eq!(get(&s, 6, 1), "=B7");
    }
}
