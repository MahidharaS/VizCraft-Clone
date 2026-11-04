//! A code editor for DAX, TMDL, Tableau calculations and M: monospace text with line numbers and
//! syntax highlighting (an egui `LayoutJob` per edit), plus the DAX function reference.

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Ui};

use crate::theme::Tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Dax,
    Tmdl,
    Tableau,
}

/// Highlight colours (light / dark).
struct Pal {
    text: Color32,
    keyword: Color32,
    function: Color32,
    string: Color32,
    number: Color32,
    comment: Color32,
    table: Color32,
    column: Color32,
    measure: Color32,
    property: Color32,
    variable: Color32,
}

fn pal(dark: bool) -> Pal {
    if dark {
        Pal {
            text: Color32::from_rgb(0xD8, 0xDE, 0xE9),
            keyword: Color32::from_rgb(0x7A, 0xB7, 0xFF),
            function: Color32::from_rgb(0xE6, 0xC0, 0x7B),
            string: Color32::from_rgb(0xF0, 0x9A, 0x7E),
            number: Color32::from_rgb(0xB5, 0xE0, 0x9A),
            comment: Color32::from_rgb(0x6F, 0xA7, 0x6A),
            table: Color32::from_rgb(0x56, 0xD0, 0xC4),
            column: Color32::from_rgb(0xC6, 0x9C, 0xF5),
            measure: Color32::from_rgb(0xF5, 0x8E, 0xC9),
            property: Color32::from_rgb(0x9C, 0xDC, 0xFE),
            variable: Color32::from_rgb(0x9C, 0xDC, 0xFE),
        }
    } else {
        Pal {
            text: Color32::from_rgb(0x1F, 0x23, 0x2B),
            keyword: Color32::from_rgb(0x00, 0x46, 0xC8),
            function: Color32::from_rgb(0x9A, 0x5B, 0x00),
            string: Color32::from_rgb(0xB3, 0x26, 0x1E),
            number: Color32::from_rgb(0x0F, 0x7B, 0x3F),
            comment: Color32::from_rgb(0x4E, 0x8A, 0x3E),
            table: Color32::from_rgb(0x00, 0x7A, 0x80),
            column: Color32::from_rgb(0x6B, 0x2F, 0xB3),
            measure: Color32::from_rgb(0xB0, 0x1E, 0x7A),
            property: Color32::from_rgb(0x1B, 0x5E, 0x9E),
            variable: Color32::from_rgb(0x00, 0x5F, 0x87),
        }
    }
}

const DAX_KEYWORDS: &[&str] = &[
    "VAR", "RETURN", "DEFINE", "EVALUATE", "MEASURE", "COLUMN", "TABLE", "ORDER", "BY", "ASC", "DESC", "START", "AT", "IN", "NOT", "AND", "OR", "TRUE", "FALSE",
    "DAY", "WEEK", "MONTH", "QUARTER", "YEAR", "HOUR", "MINUTE", "SECOND", "STRING", "INTEGER", "DOUBLE", "CURRENCY", "BOOLEAN", "DATETIME", "BOTH", "NONE", "ONEWAY",
];
const TABLEAU_KEYWORDS: &[&str] =
    &["IF", "THEN", "ELSE", "ELSEIF", "END", "CASE", "WHEN", "AND", "OR", "NOT", "FIXED", "INCLUDE", "EXCLUDE", "TRUE", "FALSE", "NULL", "IN"];
const TMDL_KEYWORDS: &[&str] = &[
    "model",
    "table",
    "measure",
    "column",
    "hierarchy",
    "level",
    "partition",
    "relationship",
    "role",
    "tablePermission",
    "member",
    "expression",
    "annotation",
    "ref",
    "createOrReplace",
    "culture",
    "perspective",
];

/// Coloured spans of `text` for `lang`. `measures` are names shown in the measure colour.
pub fn highlight(text: &str, lang: Lang, dark: bool, measures: &[String], size: f32) -> LayoutJob {
    let p = pal(dark);
    let mut job = LayoutJob::default();
    let font = FontId::monospace(size);
    let push = |job: &mut LayoutJob, s: &str, c: Color32, italic: bool| {
        job.append(s, 0.0, TextFormat { font_id: font.clone(), color: c, italics: italic, ..Default::default() });
    };
    match lang {
        Lang::Tmdl => {
            for (i, line) in text.split('\n').enumerate() {
                if i > 0 {
                    push(&mut job, "\n", p.text, false);
                }
                tmdl_line(&mut job, line, &p, measures, &push);
            }
        }
        _ => code(&mut job, text, lang, &p, measures, &push),
    }
    job
}

type Push<'a> = dyn Fn(&mut LayoutJob, &str, Color32, bool) + 'a;

fn tmdl_line(job: &mut LayoutJob, line: &str, p: &Pal, measures: &[String], push: &Push) {
    let trimmed = line.trim_start();
    let ws = &line[..line.len() - trimmed.len()];
    push(job, ws, p.text, false);
    if trimmed.starts_with("//") {
        push(job, trimmed, p.comment, trimmed.starts_with("///"));
        return;
    }
    if trimmed.starts_with("```") {
        push(job, trimmed, p.comment, false);
        return;
    }
    let word_end = trimmed.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')).unwrap_or(trimmed.len());
    let word = &trimmed[..word_end];
    let rest = &trimmed[word_end..];
    if TMDL_KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(word)) && !rest.trim_start().starts_with(':') && !word.is_empty() {
        push(job, word, p.keyword, false);
        // The name, then `= expression`.
        match rest.find('=') {
            Some(eq) if !rest[..eq].contains('\'') || rest[..eq].matches('\'').count() % 2 == 0 => {
                push(job, &rest[..eq], p.table, false);
                push(job, "=", p.text, false);
                code(job, &rest[eq + 1..], Lang::Dax, p, measures, push);
            }
            _ => push(job, rest, p.table, false),
        }
        return;
    }
    let after = rest.trim_start();
    if after.starts_with(':') && !word.is_empty() {
        let colon = rest.len() - after.len();
        push(job, word, p.property, false);
        push(job, &rest[..colon + 1], p.text, false);
        let val = &rest[colon + 1..];
        let c = if val.trim().parse::<f64>().is_ok() || matches!(val.trim(), "true" | "false") { p.number } else { p.string };
        push(job, val, c, false);
        return;
    }
    if rest.trim().is_empty() && !word.is_empty() && word.chars().next().is_some_and(|c| c.is_lowercase()) {
        push(job, word, p.property, false);
        push(job, rest, p.text, false);
        return;
    }
    if rest.trim_start().starts_with('=') && !word.is_empty() {
        push(job, word, p.property, false);
        code(job, rest, Lang::Dax, p, measures, push);
        return;
    }
    // An expression line (DAX or M).
    code(job, trimmed, Lang::Dax, p, measures, push);
}

fn code(job: &mut LayoutJob, text: &str, lang: Lang, p: &Pal, measures: &[String], push: &Push) {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let at = |i: usize| chars.get(i).map(|x| x.1);
    let off = |i: usize| chars.get(i).map(|x| x.0).unwrap_or(text.len());
    let slice = |a: usize, b: usize| &text[off(a)..off(b)];
    let keywords = if lang == Lang::Tableau { TABLEAU_KEYWORDS } else { DAX_KEYWORDS };
    let mut i = 0;
    let mut plain_start = 0;
    let flush = |job: &mut LayoutJob, a: usize, b: usize| {
        if b > a {
            push(job, slice(a, b), p.text, false);
        }
    };
    while let Some(c) = at(i) {
        let start = i;
        let (end, color, italic) = if (c == '/' && at(i + 1) == Some('/')) || (c == '-' && at(i + 1) == Some('-') && lang != Lang::Tableau) {
            let mut j = i;
            while at(j).is_some_and(|x| x != '\n') {
                j += 1;
            }
            (j, p.comment, true)
        } else if c == '/' && at(i + 1) == Some('*') {
            let mut j = i + 2;
            while at(j).is_some() && !(at(j) == Some('*') && at(j + 1) == Some('/')) {
                j += 1;
            }
            ((j + 2).min(chars.len()), p.comment, true)
        } else if c == '"' || (c == '\'' && lang == Lang::Tableau) {
            let mut j = i + 1;
            while at(j).is_some_and(|x| x != c) {
                j += 1;
            }
            ((j + 1).min(chars.len()), p.string, false)
        } else if c == '\'' {
            let mut j = i + 1;
            while at(j).is_some_and(|x| x != '\'') {
                j += 1;
            }
            ((j + 1).min(chars.len()), p.table, false)
        } else if c == '#' && lang == Lang::Tableau {
            let mut j = i + 1;
            while at(j).is_some_and(|x| x != '#') {
                j += 1;
            }
            ((j + 1).min(chars.len()), p.number, false)
        } else if c == '[' {
            let mut j = i + 1;
            while at(j).is_some_and(|x| x != ']') {
                j += 1;
            }
            let end = (j + 1).min(chars.len());
            let name = slice(i + 1, j.min(chars.len()));
            let is_measure = measures.iter().any(|m| m.eq_ignore_ascii_case(name));
            (end, if is_measure { p.measure } else { p.column }, false)
        } else if c.is_ascii_digit() || (c == '.' && at(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let mut j = i;
            while at(j).is_some_and(|x| x.is_ascii_digit() || x == '.' || x == 'e' || x == 'E') {
                j += 1;
            }
            (j, p.number, false)
        } else if c.is_alphabetic() || c == '_' || c == '@' {
            let mut j = i + 1;
            while at(j).is_some_and(|x| x.is_alphanumeric() || x == '_' || x == '.') {
                j += 1;
            }
            let word = slice(i, j);
            let mut k = j;
            while at(k).is_some_and(|x| x == ' ') {
                k += 1;
            }
            let color = if c == '@' {
                p.variable
            } else if at(k) == Some('(') {
                p.function
            } else if keywords.iter().any(|kw| kw.eq_ignore_ascii_case(word)) {
                p.keyword
            } else if word.starts_with("__") || word.starts_with('_') {
                p.variable
            } else if lang == Lang::Dax && at(j) == Some('[') {
                p.table
            } else {
                p.text
            };
            (j, color, false)
        } else {
            i += 1;
            continue;
        };
        flush(job, plain_start, start);
        push(job, slice(start, end), color, italic);
        i = end.max(start + 1);
        plain_start = i;
    }
    flush(job, plain_start, chars.len());
}

/// What the editor reports back.
pub struct EditorOut {
    #[allow(dead_code)]
    pub response: egui::Response,
    pub changed: bool,
    /// Ctrl/Cmd+Enter was pressed while the editor had focus.
    pub run: bool,
}

/// A code editor with line numbers. `height`: the visible height (it scrolls inside).
#[allow(clippy::too_many_arguments)]
pub fn code_editor(ui: &mut Ui, id: egui::Id, text: &mut String, lang: Lang, t: &Tokens, measures: &[String], height: f32, hint: &str) -> EditorOut {
    let size = 13.0;
    let dark = t.dark;
    let row_h = ui.ctx().fonts_mut(|f| f.row_height(&FontId::monospace(size)));
    let lines = text.split('\n').count().max(1);
    let bg = if dark { Color32::from_rgb(0x14, 0x17, 0x1E) } else { Color32::WHITE };
    let gutter_bg = if dark { Color32::from_rgb(0x1A, 0x1E, 0x27) } else { Color32::from_rgb(0xF6, 0xF7, 0xF9) };
    let mut run = false;
    let mut changed = false;
    let frame = egui::Frame::new().fill(bg).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(4.0);
    let inner = frame.show(ui, |ui| {
        ui.set_min_height(height);
        egui::ScrollArea::both().id_salt(id.with("scroll")).max_height(height).auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                // Gutter with line numbers.
                let digits = lines.to_string().len().max(2);
                let gw = digits as f32 * 8.0 + 16.0;
                let gh = (lines as f32 * row_h + 8.0).max(height);
                let (grect, _) = ui.allocate_exact_size(egui::vec2(gw, gh), egui::Sense::hover());
                ui.painter().rect_filled(grect, 0.0, gutter_bg);
                ui.painter().line_segment([grect.right_top(), grect.right_bottom()], egui::Stroke::new(1.0, t.border));
                let clip = ui.clip_rect();
                let ms = measures.to_vec();
                let mut layouter = move |ui: &Ui, buf: &dyn egui::TextBuffer, _wrap: f32| -> Arc<egui::Galley> {
                    let mut job = highlight(buf.as_str(), lang, dark, &ms, size);
                    job.wrap.max_width = f32::INFINITY;
                    ui.fonts_mut(|f| f.layout_job(job))
                };
                let te = egui::TextEdit::multiline(text)
                    .id(id)
                    .code_editor()
                    .lock_focus(true)
                    .desired_width(f32::INFINITY)
                    .desired_rows(((height - 8.0) / row_h).max(1.0) as usize)
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 4 }))
                    .hint_text(egui::RichText::new(hint).color(t.muted).monospace())
                    .layouter(&mut layouter);
                let out = te.show(ui);
                // Line numbers at the rows the text was laid out on.
                let mut line = 1;
                let mut new_line = true;
                for row in &out.galley.rows {
                    let y = out.galley_pos.y + row.pos.y;
                    if new_line && y + row_h >= clip.top() && y <= clip.bottom() {
                        ui.painter().text(egui::pos2(grect.right() - 7.0, y + row.rect().height() / 2.0), egui::Align2::RIGHT_CENTER, line.to_string(), FontId::monospace(size - 1.5), t.muted);
                    }
                    new_line = row.ends_with_newline;
                    if new_line {
                        line += 1;
                    }
                }
                changed = out.response.changed();
                if out.response.has_focus() {
                    run = ui.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Enter)));
                }
                out.response.response
            })
            .inner
        })
        .inner
    });
    EditorOut { response: inner.inner, changed, run }
}

/// Inserts `snippet` at the editor's cursor (or at the end).
pub fn insert_at_cursor(ctx: &egui::Context, id: egui::Id, text: &mut String, snippet: &str) {
    let mut state = egui::text_edit::TextEditState::load(ctx, id);
    let pos = state.as_ref().and_then(|s| s.cursor.char_range()).map(|r| r.primary.index.0).unwrap_or(text.chars().count());
    let byte = text.char_indices().nth(pos).map(|(b, _)| b).unwrap_or(text.len());
    text.insert_str(byte, snippet);
    if let Some(s) = state.as_mut() {
        let c = egui::text::CCursor::new(pos + snippet.chars().count());
        s.cursor.set_char_range(Some(egui::text::CCursorRange::one(c)));
        s.clone().store(ctx, id);
    }
    ctx.memory_mut(|m| m.request_focus(id));
}

/// DAX functions the engine knows: name, arguments, what it does.
pub const DAX_FUNCTIONS: &[(&str, &str, &str, &str)] = &[
    // (category, name, signature, description)
    ("Aggregation", "SUM", "SUM(<column>)", "Adds all the numbers in a column."),
    ("Aggregation", "AVERAGE", "AVERAGE(<column>)", "The arithmetic mean of a column."),
    ("Aggregation", "MIN", "MIN(<column>)", "The smallest value of a column (or of two values)."),
    ("Aggregation", "MAX", "MAX(<column>)", "The largest value of a column (or of two values)."),
    ("Aggregation", "COUNT", "COUNT(<column>)", "Counts the numbers in a column."),
    ("Aggregation", "COUNTA", "COUNTA(<column>)", "Counts the non-blank values of a column."),
    ("Aggregation", "COUNTBLANK", "COUNTBLANK(<column>)", "Counts the blank values of a column."),
    ("Aggregation", "COUNTROWS", "COUNTROWS(<table>)", "Counts the rows of a table."),
    ("Aggregation", "DISTINCTCOUNT", "DISTINCTCOUNT(<column>)", "Counts the distinct values of a column."),
    ("Aggregation", "MEDIAN", "MEDIAN(<column>)", "The median of a column."),
    ("Aggregation", "PRODUCT", "PRODUCT(<column>)", "Multiplies the numbers of a column."),
    ("Aggregation", "MAXA", "MAXA(<column>)", "Largest value, counting TRUE as 1."),
    ("Aggregation", "MINA", "MINA(<column>)", "Smallest value, counting TRUE as 1."),
    ("Aggregation", "SUMX", "SUMX(<table>, <expression>)", "Sums an expression evaluated for each row."),
    ("Aggregation", "AVERAGEX", "AVERAGEX(<table>, <expression>)", "Averages an expression over a table."),
    ("Aggregation", "MINX", "MINX(<table>, <expression>)", "Smallest value of an expression over a table."),
    ("Aggregation", "MAXX", "MAXX(<table>, <expression>)", "Largest value of an expression over a table."),
    ("Aggregation", "COUNTX", "COUNTX(<table>, <expression>)", "Counts the numeric results of an expression."),
    ("Aggregation", "CONCATENATEX", "CONCATENATEX(<table>, <expression>, [delimiter])", "Joins an expression's results as text."),
    ("Aggregation", "RANKX", "RANKX(<table>, <expression>)", "The rank of the current value among the table's."),
    ("Statistical", "STDEV.S", "STDEV.S(<column>)", "Standard deviation of a sample."),
    ("Statistical", "STDEV.P", "STDEV.P(<column>)", "Standard deviation of a population."),
    ("Statistical", "VAR.S", "VAR.S(<column>)", "Variance of a sample."),
    ("Statistical", "VAR.P", "VAR.P(<column>)", "Variance of a population."),
    ("Statistical", "STDEVX.P", "STDEVX.P(<table>, <expression>)", "Population standard deviation of an expression."),
    ("Statistical", "VARX.P", "VARX.P(<table>, <expression>)", "Population variance of an expression."),
    ("Statistical", "PERCENTILE.INC", "PERCENTILE.INC(<column>, <k>)", "The k-th percentile (inclusive)."),
    ("Statistical", "PERCENTILE.EXC", "PERCENTILE.EXC(<column>, <k>)", "The k-th percentile (exclusive)."),
    ("Statistical", "PERCENTILEX.INC", "PERCENTILEX.INC(<table>, <expression>, <k>)", "Percentile of an expression."),
    ("Statistical", "RANK.EQ", "RANK.EQ(<value>, <column>, [order])", "The rank of a value in a column."),
    ("Filter", "CALCULATE", "CALCULATE(<expression>, <filter>, …)", "Evaluates an expression in a modified filter context."),
    ("Filter", "CALCULATETABLE", "CALCULATETABLE(<table>, <filter>, …)", "A table in a modified filter context."),
    ("Filter", "FILTER", "FILTER(<table>, <condition>)", "The rows of a table that meet a condition."),
    ("Filter", "ALL", "ALL(<table or columns>)", "Ignores filters on a table or columns."),
    ("Filter", "ALLEXCEPT", "ALLEXCEPT(<table>, <column>, …)", "Removes all filters on a table except those on the columns."),
    ("Filter", "ALLSELECTED", "ALLSELECTED([table or column])", "Removes filters from inside the visual, keeping outside ones."),
    ("Filter", "REMOVEFILTERS", "REMOVEFILTERS([table or column])", "Clears filters (a CALCULATE modifier)."),
    ("Filter", "KEEPFILTERS", "KEEPFILTERS(<filter>)", "Intersects a filter with the existing ones."),
    ("Filter", "VALUES", "VALUES(<column or table>)", "The distinct values in the current context."),
    ("Filter", "DISTINCT", "DISTINCT(<column or table>)", "The distinct values (no blank row)."),
    ("Filter", "SELECTEDVALUE", "SELECTEDVALUE(<column>, [alternate])", "The value when one value is filtered, else the alternate."),
    ("Filter", "HASONEVALUE", "HASONEVALUE(<column>)", "TRUE when the column is filtered to one value."),
    ("Filter", "ISFILTERED", "ISFILTERED(<column or table>)", "TRUE when the column is filtered directly."),
    ("Filter", "ISCROSSFILTERED", "ISCROSSFILTERED(<column or table>)", "TRUE when filtered directly or by a related table."),
    ("Filter", "ISINSCOPE", "ISINSCOPE(<column>)", "TRUE when the column is a grouping level in the current cell."),
    ("Filter", "USERELATIONSHIP", "USERELATIONSHIP(<column1>, <column2>)", "Uses an inactive relationship in CALCULATE."),
    ("Filter", "CROSSFILTER", "CROSSFILTER(<column1>, <column2>, <direction>)", "Changes cross-filtering (kept as the model's)."),
    ("Filter", "TREATAS", "TREATAS(<table>, <column>, …)", "Applies a table's values as filters on columns."),
    ("Filter", "EARLIER", "EARLIER(<column>, [n])", "The column's value in an outer row context."),
    ("Filter", "LOOKUPVALUE", "LOOKUPVALUE(<result>, <search column>, <value>, …)", "The value of a column in the row matching the search."),
    ("Filter", "RELATED", "RELATED(<column>)", "A value from the lookup table of the current row."),
    ("Filter", "RELATEDTABLE", "RELATEDTABLE(<table>)", "The rows related to the current row."),
    ("Filter", "CONTAINS", "CONTAINS(<table>, <column>, <value>, …)", "TRUE when a row has the values."),
    ("Filter", "FIRSTNONBLANK", "FIRSTNONBLANK(<column>, <expression>)", "First value of the column where the expression isn't blank."),
    ("Filter", "LASTNONBLANK", "LASTNONBLANK(<column>, <expression>)", "Last value of the column where the expression isn't blank."),
    ("Table", "SUMMARIZECOLUMNS", "SUMMARIZECOLUMNS(<group by>, …, <name>, <expression>, …)", "A summary table grouped by columns."),
    ("Table", "SUMMARIZE", "SUMMARIZE(<table>, <group by>, …, [name, expression])", "A summary of a table grouped by columns."),
    ("Table", "ADDCOLUMNS", "ADDCOLUMNS(<table>, <name>, <expression>, …)", "Adds calculated columns to a table."),
    ("Table", "SELECTCOLUMNS", "SELECTCOLUMNS(<table>, <name>, <expression>, …)", "A table of chosen expressions."),
    ("Table", "TOPN", "TOPN(<n>, <table>, <order by>, [order])", "The top n rows of a table."),
    ("Table", "GENERATESERIES", "GENERATESERIES(<start>, <end>, [step])", "A one-column table of numbers."),
    ("Table", "CALENDAR", "CALENDAR(<start date>, <end date>)", "A table of every date in a range."),
    ("Table", "CALENDARAUTO", "CALENDARAUTO([fiscal year end month])", "A date table covering the model's dates."),
    ("Table", "UNION", "UNION(<table>, <table>, …)", "Rows of several tables."),
    ("Table", "INTERSECT", "INTERSECT(<table>, <table>)", "Rows in both tables."),
    ("Table", "EXCEPT", "EXCEPT(<table>, <table>)", "Rows of the first table not in the second."),
    ("Table", "CROSSJOIN", "CROSSJOIN(<table>, <table>, …)", "Every combination of the tables' rows."),
    ("Table", "NATURALINNERJOIN", "NATURALINNERJOIN(<table>, <table>)", "Joins tables on their common columns."),
    ("Table", "GENERATE", "GENERATE(<table>, <table expression>)", "For each row, the rows of a table expression."),
    ("Table", "ROW", "ROW(<name>, <expression>, …)", "A table with one row."),
    ("Table", "DATATABLE", "DATATABLE(<name>, <type>, …, {{…}})", "A table of literal values."),
    ("Table", "SAMPLE", "SAMPLE(<n>, <table>, <order by>)", "Evenly spaced rows of a table."),
    ("Time intelligence", "TOTALYTD", "TOTALYTD(<expression>, <dates>)", "Year-to-date total."),
    ("Time intelligence", "TOTALQTD", "TOTALQTD(<expression>, <dates>)", "Quarter-to-date total."),
    ("Time intelligence", "TOTALMTD", "TOTALMTD(<expression>, <dates>)", "Month-to-date total."),
    ("Time intelligence", "DATESYTD", "DATESYTD(<dates>)", "Dates from the start of the year."),
    ("Time intelligence", "SAMEPERIODLASTYEAR", "SAMEPERIODLASTYEAR(<dates>)", "The same dates one year earlier."),
    ("Time intelligence", "DATEADD", "DATEADD(<dates>, <n>, <interval>)", "Dates shifted by an interval."),
    ("Time intelligence", "DATESBETWEEN", "DATESBETWEEN(<dates>, <start>, <end>)", "Dates in a range."),
    ("Time intelligence", "DATESINPERIOD", "DATESINPERIOD(<dates>, <start>, <n>, <interval>)", "Dates in a period from a start date."),
    ("Time intelligence", "PARALLELPERIOD", "PARALLELPERIOD(<dates>, <n>, <interval>)", "The whole parallel period, shifted."),
    ("Time intelligence", "PREVIOUSMONTH", "PREVIOUSMONTH(<dates>)", "The dates of the previous month."),
    ("Time intelligence", "PREVIOUSYEAR", "PREVIOUSYEAR(<dates>)", "The dates of the previous year."),
    ("Time intelligence", "NEXTMONTH", "NEXTMONTH(<dates>)", "The dates of the next month."),
    ("Time intelligence", "NEXTYEAR", "NEXTYEAR(<dates>)", "The dates of the next year."),
    ("Time intelligence", "STARTOFMONTH", "STARTOFMONTH(<dates>)", "The first date of the month."),
    ("Time intelligence", "ENDOFMONTH", "ENDOFMONTH(<dates>)", "The last date of the month."),
    ("Time intelligence", "FIRSTDATE", "FIRSTDATE(<dates>)", "The first date in the context."),
    ("Time intelligence", "LASTDATE", "LASTDATE(<dates>)", "The last date in the context."),
    ("Date", "DATE", "DATE(<year>, <month>, <day>)", "A date."),
    ("Date", "TODAY", "TODAY()", "Today's date."),
    ("Date", "NOW", "NOW()", "The current date and time."),
    ("Date", "YEAR", "YEAR(<date>)", "The year of a date."),
    ("Date", "QUARTER", "QUARTER(<date>)", "The quarter (1–4) of a date."),
    ("Date", "MONTH", "MONTH(<date>)", "The month (1–12) of a date."),
    ("Date", "DAY", "DAY(<date>)", "The day of the month."),
    ("Date", "WEEKDAY", "WEEKDAY(<date>, [return type])", "The day of the week."),
    ("Date", "WEEKNUM", "WEEKNUM(<date>, [return type])", "The week number of the year."),
    ("Date", "EOMONTH", "EOMONTH(<date>, <months>)", "The last day of a month."),
    ("Date", "EDATE", "EDATE(<date>, <months>)", "A date some months away."),
    ("Date", "DATEDIFF", "DATEDIFF(<date1>, <date2>, <interval>)", "Interval boundaries between two dates."),
    ("Logical", "IF", "IF(<condition>, <then>, [else])", "One value or another."),
    ("Logical", "SWITCH", "SWITCH(<expression>, <value>, <result>, …, [else])", "The result matching a value."),
    ("Logical", "IFERROR", "IFERROR(<value>, <value if error>)", "A fallback when an expression fails."),
    ("Logical", "COALESCE", "COALESCE(<value>, <value>, …)", "The first non-blank value."),
    ("Logical", "AND", "AND(<a>, <b>)", "TRUE when both are TRUE."),
    ("Logical", "OR", "OR(<a>, <b>)", "TRUE when either is TRUE."),
    ("Logical", "NOT", "NOT(<value>)", "The opposite truth value."),
    ("Information", "ISBLANK", "ISBLANK(<value>)", "TRUE when the value is blank."),
    ("Information", "ISERROR", "ISERROR(<value>)", "TRUE when the value is an error."),
    ("Information", "ISNUMBER", "ISNUMBER(<value>)", "TRUE for numbers."),
    ("Information", "ISTEXT", "ISTEXT(<value>)", "TRUE for text."),
    ("Information", "ISEMPTY", "ISEMPTY(<table>)", "TRUE when the table has no rows."),
    ("Information", "ISONORAFTER", "ISONORAFTER(<value>, <value>, [order], …)", "Compares values in sort order."),
    ("Math", "DIVIDE", "DIVIDE(<numerator>, <denominator>, [alternate])", "Division that handles division by zero."),
    ("Math", "ROUND", "ROUND(<number>, <digits>)", "Rounds a number."),
    ("Math", "ROUNDUP", "ROUNDUP(<number>, <digits>)", "Rounds away from zero."),
    ("Math", "ROUNDDOWN", "ROUNDDOWN(<number>, <digits>)", "Rounds toward zero."),
    ("Math", "INT", "INT(<number>)", "Rounds down to an integer."),
    ("Math", "ABS", "ABS(<number>)", "The absolute value."),
    ("Math", "MOD", "MOD(<number>, <divisor>)", "The remainder of a division."),
    ("Math", "QUOTIENT", "QUOTIENT(<numerator>, <denominator>)", "The integer part of a division."),
    ("Math", "POWER", "POWER(<number>, <power>)", "A number raised to a power."),
    ("Math", "SQRT", "SQRT(<number>)", "The square root."),
    ("Math", "CEILING", "CEILING(<number>, <significance>)", "Rounds up to a multiple."),
    ("Math", "FLOOR", "FLOOR(<number>, <significance>)", "Rounds down to a multiple."),
    ("Math", "MROUND", "MROUND(<number>, <multiple>)", "Rounds to the nearest multiple."),
    ("Math", "EVEN", "EVEN(<number>)", "Rounds up to an even integer."),
    ("Math", "ODD", "ODD(<number>)", "Rounds up to an odd integer."),
    ("Math", "GCD", "GCD(<number>, …)", "The greatest common divisor."),
    ("Math", "LCM", "LCM(<number>, …)", "The least common multiple."),
    ("Math", "RAND", "RAND()", "A random number in [0, 1)."),
    ("Math", "RANDBETWEEN", "RANDBETWEEN(<bottom>, <top>)", "A random integer in a range."),
    ("Text", "CONCATENATE", "CONCATENATE(<text1>, <text2>)", "Joins two texts."),
    ("Text", "COMBINEVALUES", "COMBINEVALUES(<delimiter>, <value>, …)", "Joins values with a delimiter."),
    ("Text", "FORMAT", "FORMAT(<value>, <format>)", "A value as text in a format (\"0.0%\", \"mmm yyyy\")."),
    ("Text", "FIXED", "FIXED(<number>, [decimals], [no commas])", "A number as text with fixed decimals."),
    ("Text", "LEFT", "LEFT(<text>, [n])", "The first characters."),
    ("Text", "RIGHT", "RIGHT(<text>, [n])", "The last characters."),
    ("Text", "MID", "MID(<text>, <start>, <n>)", "Characters from the middle."),
    ("Text", "LEN", "LEN(<text>)", "The number of characters."),
    ("Text", "UPPER", "UPPER(<text>)", "Upper case."),
    ("Text", "LOWER", "LOWER(<text>)", "Lower case."),
    ("Text", "TRIM", "TRIM(<text>)", "Removes extra spaces."),
    ("Text", "SUBSTITUTE", "SUBSTITUTE(<text>, <old>, <new>)", "Replaces text."),
    ("Text", "REPLACE", "REPLACE(<text>, <start>, <n>, <new>)", "Replaces characters by position."),
    ("Text", "SEARCH", "SEARCH(<find>, <within>, [start], [not found])", "Position of text (case-insensitive)."),
    ("Text", "FIND", "FIND(<find>, <within>, [start], [not found])", "Position of text (case-sensitive)."),
    ("Text", "CONTAINSSTRING", "CONTAINSSTRING(<text>, <find>)", "TRUE when text contains another."),
    ("Text", "REPT", "REPT(<text>, <n>)", "Repeats text."),
    ("Text", "UNICHAR", "UNICHAR(<code>)", "The character for a Unicode code."),
    ("Text", "VALUE", "VALUE(<text>)", "Converts text to a number."),
    ("Text", "PATHITEM", "PATHITEM(<path>, <position>)", "An item of a | separated path."),
];

/// The function reference: a searchable list; returns a signature to insert when one is clicked.
pub fn function_list(ui: &mut Ui, t: &Tokens, search: &mut String) -> Option<String> {
    let mut pick = None;
    ui.add(egui::TextEdit::singleline(search).hint_text("Search functions").desired_width(f32::INFINITY));
    ui.add_space(4.0);
    let q = search.to_lowercase();
    egui::ScrollArea::vertical().id_salt("fnlist").auto_shrink([false, false]).show(ui, |ui| {
        let mut cat = "";
        for (c, name, sig, d) in DAX_FUNCTIONS {
            if !q.is_empty() && !name.to_lowercase().contains(&q) && !d.to_lowercase().contains(&q) {
                continue;
            }
            if *c != cat {
                cat = c;
                ui.add_space(4.0);
                ui.label(egui::RichText::new(c.to_uppercase()).size(10.0).color(t.muted).strong());
            }
            let r = ui.add(egui::Button::new(egui::RichText::new(*name).monospace().size(12.0).color(t.text)).frame(false)).on_hover_ui(|ui| {
                ui.label(egui::RichText::new(*sig).monospace().strong());
                ui.label(*d);
            });
            if r.clicked() {
                // Insert the call with its parentheses.
                pick = Some(format!("{name}("));
            }
        }
    });
    pick
}
