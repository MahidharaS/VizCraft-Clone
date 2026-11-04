//! SQL text: statement splitting (quotes and comments aware) and a tokenizer for highlighting.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok {
    Keyword,
    Function,
    Str,
    Number,
    Comment,
    /// A quoted identifier (`"Sales"`).
    QuotedIdent,
    Ident,
    Operator,
    Punct,
    Space,
}

pub const KEYWORDS: &[&str] = &[
    "ADD", "ALL", "ALTER", "ANALYZE", "AND", "ANTI", "ANY", "AS", "ASC", "ASOF", "ATTACH", "BETWEEN", "BY", "CASE", "CAST", "CHECKPOINT", "COLUMN", "COLUMNS", "COPY",
    "CREATE", "CROSS", "CUBE", "CURRENT", "DATABASE", "DEFAULT", "DELETE", "DESC", "DESCRIBE", "DETACH", "DISTINCT", "DROP", "ELSE", "END", "EXCEPT", "EXCLUDE",
    "EXISTS", "EXPLAIN", "EXPORT", "FALSE", "FILTER", "FIRST", "FOLLOWING", "FOR", "FORMAT", "FROM", "FULL", "GROUP", "GROUPING", "HAVING", "IF", "ILIKE", "IMPORT",
    "IN", "INNER", "INSERT", "INSTALL", "INTERSECT", "INTERVAL", "INTO", "IS", "JOIN", "LAST", "LATERAL", "LEFT", "LIKE", "LIMIT", "LOAD", "MACRO", "NATURAL", "NOT",
    "NULL", "NULLS", "OFFSET", "ON", "OR", "ORDER", "OUTER", "OVER", "PARTITION", "PIVOT", "POSITIONAL", "PRAGMA", "PRECEDING", "PRIMARY", "QUALIFY", "RANGE",
    "RECURSIVE", "REPLACE", "RETURNING", "RIGHT", "ROLLUP", "ROW", "ROWS", "SAMPLE", "SCHEMA", "SELECT", "SEMI", "SEQUENCE", "SET", "SETS", "SHOW", "SUMMARIZE",
    "TABLE", "TABLES", "TEMP", "TEMPORARY", "THEN", "TO", "TRUE", "TYPE", "UNBOUNDED", "UNION", "UNPIVOT", "UPDATE", "USE", "USING", "VACUUM", "VALUES", "VIEW",
    "WHEN", "WHERE", "WINDOW", "WITH", "BIGINT", "BOOLEAN", "DATE", "DECIMAL", "DOUBLE", "INTEGER", "TIMESTAMP", "TIME", "VARCHAR", "TOP",
];

pub const FUNCTIONS: &[&str] = &[
    "abs", "any_value", "approx_count_distinct", "approx_quantile", "arg_max", "arg_min", "array_agg", "avg", "bit_and", "bool_and", "ceil", "coalesce", "concat",
    "concat_ws", "contains", "corr", "count", "covar_pop", "cume_dist", "current_date", "current_timestamp", "date_add", "date_diff", "date_part", "date_sub",
    "date_trunc", "datediff", "datepart", "dayname", "dayofweek", "dense_rank", "epoch", "exp", "extract", "first", "first_value", "floor", "generate_series",
    "greatest", "hash", "histogram", "if", "ifnull", "lag", "last", "last_day", "last_value", "lead", "least", "left", "length", "list", "list_agg", "ln", "log",
    "lower", "lpad", "ltrim", "make_date", "max", "max_by", "md5", "median", "min", "min_by", "mode", "month", "monthname", "now", "nth_value", "ntile", "nullif",
    "percent_rank", "percentile_cont", "percentile_disc", "pow", "power", "quantile", "quantile_cont", "random", "range", "rank", "read_csv", "read_csv_auto",
    "read_json", "read_json_auto", "read_parquet", "regexp_extract", "regexp_matches", "regexp_replace", "repeat", "replace", "reverse", "right", "round",
    "row_number", "rpad", "rtrim", "split_part", "sqrt", "starts_with", "stddev", "stddev_pop", "stddev_samp", "strftime", "string_agg", "strptime", "substr",
    "substring", "sum", "time_bucket", "to_timestamp", "trim", "trunc", "try_cast", "unnest", "upper", "var_pop", "var_samp", "variance", "week", "year", "quarter",
    "day", "hour", "minute", "second", "strlen", "list_value", "struct_pack", "read_xlsx", "sqlite_scan", "postgres_scan", "mysql_scan", "glob",
];

pub fn is_keyword(w: &str) -> bool {
    KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(w))
}
pub fn is_function(w: &str) -> bool {
    FUNCTIONS.iter().any(|k| k.eq_ignore_ascii_case(w))
}

/// Tokens with byte ranges covering the whole text.
pub fn tokenize(s: &str) -> Vec<(Range<usize>, Tok)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let start = i;
        let kind = if c == b'-' && b.get(i + 1) == Some(&b'-') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            Tok::Comment
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i = (i + 2).min(b.len());
            Tok::Comment
        } else if c == b'\'' || c == b'"' {
            i += 1;
            while i < b.len() {
                if b[i] == c {
                    if b.get(i + 1) == Some(&c) {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            if c == b'\'' { Tok::Str } else { Tok::QuotedIdent }
        } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.' || b[i] == b'_') {
                i += 1;
            }
            Tok::Number
        } else if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] >= 0x80) {
                i += 1;
            }
            let w = &s[start..i];
            // A word followed by `(` is a function call.
            let mut j = i;
            while j < b.len() && b[j] == b' ' {
                j += 1;
            }
            if b.get(j) == Some(&b'(') && (is_function(w) || !is_keyword(w)) {
                Tok::Function
            } else if is_keyword(w) {
                Tok::Keyword
            } else {
                Tok::Ident
            }
        } else if c.is_ascii_whitespace() {
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            Tok::Space
        } else if b"=<>!+-*/%|&^~:".contains(&c) {
            i += 1;
            Tok::Operator
        } else {
            i += 1;
            Tok::Punct
        };
        // Keep ranges on char boundaries.
        while i < s.len() && !s.is_char_boundary(i) {
            i += 1;
        }
        out.push((start..i, kind));
    }
    out
}

/// Splits a script into statements (byte ranges, trimmed, without the `;`), skipping empty ones
/// and ones that are only comments.
pub fn split_statements(s: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let toks = tokenize(s);
    for (r, k) in &toks {
        if *k == Tok::Punct && &s[r.clone()] == ";" {
            push_stmt(s, start..r.start, &toks, &mut out);
            start = r.end;
        }
    }
    push_stmt(s, start..s.len(), &toks, &mut out);
    out
}

fn push_stmt(s: &str, r: Range<usize>, toks: &[(Range<usize>, Tok)], out: &mut Vec<Range<usize>>) {
    // Trim to the first / last meaningful token.
    let inside: Vec<&(Range<usize>, Tok)> = toks.iter().filter(|(t, k)| t.start >= r.start && t.end <= r.end && !matches!(k, Tok::Space | Tok::Comment)).collect();
    if let (Some(a), Some(b)) = (inside.first(), inside.last()) {
        let _ = s;
        out.push(a.0.start..b.0.end);
    }
}

/// The statement under the cursor (byte offset): the one containing it, else the nearest before.
pub fn statement_at(s: &str, cursor: usize) -> Option<Range<usize>> {
    let stmts = split_statements(s);
    for (i, r) in stmts.iter().enumerate() {
        let next = stmts.get(i + 1).map(|n| n.start).unwrap_or(usize::MAX);
        if cursor < next {
            return Some(r.clone());
        }
    }
    stmts.last().cloned()
}

/// The word (identifier prefix) that ends at `cursor`, for autocomplete: (start, text).
pub fn word_before(s: &str, cursor: usize) -> (usize, &str) {
    let cursor = cursor.min(s.len());
    let mut i = cursor;
    let b = s.as_bytes();
    while i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_' || b[i - 1] >= 0x80) {
        i -= 1;
    }
    while i < cursor && !s.is_char_boundary(i) {
        i += 1;
    }
    (i, &s[i..cursor])
}

/// The identifier before a `.` that precedes the word at `cursor` (`Sales.` → `Sales`).
pub fn qualifier_before(s: &str, word_start: usize) -> Option<String> {
    let b = s.as_bytes();
    if word_start == 0 || b.get(word_start - 1) != Some(&b'.') {
        return None;
    }
    let end = word_start - 1;
    if end > 0 && b[end - 1] == b'"' {
        let open = s[..end - 1].rfind('"')?;
        return Some(s[open + 1..end - 1].to_string());
    }
    let (st, w) = word_before(s, end);
    let _ = st;
    if w.is_empty() { None } else { Some(w.to_string()) }
}

/// Whether a statement returns a plan.
pub fn is_explain(sql: &str) -> bool {
    first_word(sql).eq_ignore_ascii_case("explain")
}

/// The first keyword of a statement (after comments).
pub fn first_word(sql: &str) -> &str {
    for (r, k) in tokenize(sql) {
        match k {
            Tok::Space | Tok::Comment => continue,
            _ => return &sql[r],
        }
    }
    ""
}

/// Rewrites `SELECT TOP n …` (SQL Server style) to `SELECT … LIMIT n` so habits carry over.
pub fn rewrite_top(sql: &str) -> String {
    let toks: Vec<(Range<usize>, Tok)> = tokenize(sql).into_iter().filter(|(_, k)| !matches!(k, Tok::Space | Tok::Comment)).collect();
    let (Some(a), Some(b), Some(c)) = (toks.first(), toks.get(1), toks.get(2)) else { return sql.to_string() };
    if sql[a.0.clone()].eq_ignore_ascii_case("select") && sql[b.0.clone()].eq_ignore_ascii_case("top") && c.1 == Tok::Number && !sql.to_ascii_lowercase().contains(" limit ") {
        let n = &sql[c.0.clone()];
        return format!("{}{} LIMIT {}", &sql[..b.0.start], sql[c.0.end..].trim(), n);
    }
    sql.to_string()
}
