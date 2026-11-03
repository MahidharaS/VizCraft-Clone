//! The editor's ribbon and the M each action adds.
//!
//! Column operations (format, extract, number, date and time transforms) come from one catalog,
//! [`OPS`]: on the Transform tab they change the selected columns in place
//! (`Table.TransformColumns`), on the Add Column tab they add a new column (`Table.AddColumn`).

use egui::Ui;
use vizcraft_query::MType;
use vizcraft_query::steps::{field, text_list};
use vizcraft_query::value::quote_text;

use super::dialogs::{Dialog, KeepRemove, SplitMode};
use super::forms::{group, small_tool, stack, tool};
use super::{add_step, open_editor, preview_table};
use crate::app::VizApp;
use crate::theme::Tokens;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Text,
    Number,
    Date,
    Time,
    Duration,
}

/// A column operation. `f` uses `X` for the value and `P` for the parameter.
pub struct ColOp {
    pub menu: &'static str,
    pub label: &'static str,
    pub kind: Kind,
    pub f: &'static str,
    pub ty: &'static str,
    /// The prompt, default and whether the parameter is text (quoted).
    pub param: Option<(&'static str, &'static str, bool)>,
}

const fn op(menu: &'static str, label: &'static str, kind: Kind, f: &'static str, ty: &'static str) -> ColOp {
    ColOp { menu, label, kind, f, ty, param: None }
}

const fn opp(menu: &'static str, label: &'static str, kind: Kind, f: &'static str, ty: &'static str, param: (&'static str, &'static str, bool)) -> ColOp {
    ColOp { menu, label, kind, f, ty, param: Some(param) }
}

use Kind::{Date, Duration as Dur, Number as Num, Text as Txt, Time};

pub const OPS: &[ColOp] = &[
    op("Format", "lowercase", Txt, "Text.Lower(X)", "type text"),
    op("Format", "UPPERCASE", Txt, "Text.Upper(X)", "type text"),
    op("Format", "Capitalize Each Word", Txt, "Text.Proper(X)", "type text"),
    op("Format", "Trim", Txt, "Text.Trim(X)", "type text"),
    op("Format", "Clean", Txt, "Text.Clean(X)", "type text"),
    opp("Format", "Add Prefix…", Txt, "P & X", "type text", ("Prefix", "", true)),
    opp("Format", "Add Suffix…", Txt, "X & P", "type text", ("Suffix", "", true)),
    op("Extract", "Length", Txt, "Text.Length(X)", "Int64.Type"),
    opp("Extract", "First Characters…", Txt, "Text.Start(X, P)", "type text", ("Count", "1", false)),
    opp("Extract", "Last Characters…", Txt, "Text.End(X, P)", "type text", ("Count", "1", false)),
    opp("Extract", "Range…", Txt, "Text.Middle(X, P)", "type text", ("Starting index, number of characters", "0, 1", false)),
    opp("Extract", "Text Before Delimiter…", Txt, "Text.BeforeDelimiter(X, P)", "type text", ("Delimiter", " ", true)),
    opp("Extract", "Text After Delimiter…", Txt, "Text.AfterDelimiter(X, P)", "type text", ("Delimiter", " ", true)),
    opp("Extract", "Text Between Delimiters…", Txt, "Text.BetweenDelimiters(X, P)", "type text", ("Start and end delimiters (M)", "\"(\", \")\"", false)),
    op("Extract", "Digits Only", Txt, "Text.Select(X, {\"0\"..\"9\"})", "type text"),
    op("Parse", "JSON", Txt, "Json.Document(X)", "type any"),
    op("Parse", "XML", Txt, "Xml.Tables(X)", "type any"),
    opp("Standard", "Add…", Num, "X + P", "type number", ("Value", "1", false)),
    opp("Standard", "Multiply…", Num, "X * P", "type number", ("Value", "1", false)),
    opp("Standard", "Subtract…", Num, "X - P", "type number", ("Value", "1", false)),
    opp("Standard", "Divide…", Num, "X / P", "type number", ("Value", "1", false)),
    opp("Standard", "Integer-Divide…", Num, "Number.IntegerDivide(X, P)", "Int64.Type", ("Value", "1", false)),
    opp("Standard", "Modulo…", Num, "Number.Mod(X, P)", "type number", ("Value", "2", false)),
    opp("Standard", "Percentage…", Num, "X * P / 100", "type number", ("Percentage", "10", false)),
    opp("Standard", "Percent Of…", Num, "X / P * 100", "type number", ("Value", "100", false)),
    op("Scientific", "Absolute Value", Num, "Number.Abs(X)", "type number"),
    opp("Scientific", "Power…", Num, "Number.Power(X, P)", "type number", ("Power", "2", false)),
    op("Scientific", "Square Root", Num, "Number.Sqrt(X)", "type number"),
    op("Scientific", "Exponent", Num, "Number.Exp(X)", "type number"),
    op("Scientific", "Logarithm Base-10", Num, "Number.Log10(X)", "type number"),
    op("Scientific", "Natural Logarithm", Num, "Number.Ln(X)", "type number"),
    op("Scientific", "Factorial", Num, "Number.Factorial(X)", "type number"),
    op("Trigonometry", "Sine", Num, "Number.Sin(X)", "type number"),
    op("Trigonometry", "Cosine", Num, "Number.Cos(X)", "type number"),
    op("Trigonometry", "Tangent", Num, "Number.Tan(X)", "type number"),
    op("Trigonometry", "Arcsine", Num, "Number.Asin(X)", "type number"),
    op("Trigonometry", "Arccosine", Num, "Number.Acos(X)", "type number"),
    op("Trigonometry", "Arctangent", Num, "Number.Atan(X)", "type number"),
    op("Rounding", "Round Up", Num, "Number.RoundUp(X)", "Int64.Type"),
    op("Rounding", "Round Down", Num, "Number.RoundDown(X)", "Int64.Type"),
    opp("Rounding", "Round…", Num, "Number.Round(X, P)", "type number", ("Decimal places", "2", false)),
    op("Information", "Is Even", Num, "Number.IsEven(X)", "type logical"),
    op("Information", "Is Odd", Num, "Number.IsOdd(X)", "type logical"),
    op("Information", "Sign", Num, "Number.Sign(X)", "Int64.Type"),
    op("Date", "Date Only", Date, "DateTime.Date(X)", "type date"),
    op("Date", "Parse", Date, "Date.From(X)", "type date"),
    op("Date", "Age", Date, "DateTime.Date(DateTime.LocalNow()) - X", "type duration"),
    op("Date", "Year", Date, "Date.Year(X)", "Int64.Type"),
    op("Date", "Start of Year", Date, "Date.StartOfYear(X)", "type date"),
    op("Date", "End of Year", Date, "Date.EndOfYear(X)", "type date"),
    op("Date", "Month", Date, "Date.Month(X)", "Int64.Type"),
    op("Date", "Name of Month", Date, "Date.MonthName(X)", "type text"),
    op("Date", "Start of Month", Date, "Date.StartOfMonth(X)", "type date"),
    op("Date", "End of Month", Date, "Date.EndOfMonth(X)", "type date"),
    op("Date", "Days in Month", Date, "Date.DaysInMonth(X)", "Int64.Type"),
    op("Date", "Quarter of Year", Date, "Date.QuarterOfYear(X)", "Int64.Type"),
    op("Date", "Start of Quarter", Date, "Date.StartOfQuarter(X)", "type date"),
    op("Date", "End of Quarter", Date, "Date.EndOfQuarter(X)", "type date"),
    op("Date", "Week of Year", Date, "Date.WeekOfYear(X)", "Int64.Type"),
    op("Date", "Week of Month", Date, "Date.WeekOfMonth(X)", "Int64.Type"),
    op("Date", "Start of Week", Date, "Date.StartOfWeek(X)", "type date"),
    op("Date", "End of Week", Date, "Date.EndOfWeek(X)", "type date"),
    op("Date", "Day", Date, "Date.Day(X)", "Int64.Type"),
    op("Date", "Day of Week", Date, "Date.DayOfWeek(X)", "Int64.Type"),
    op("Date", "Day of Year", Date, "Date.DayOfYear(X)", "Int64.Type"),
    op("Date", "Name of Day", Date, "Date.DayOfWeekName(X)", "type text"),
    op("Date", "Year-Month", Date, "Date.ToText(X, \"yyyy-MM\")", "type text"),
    op("Time", "Time Only", Time, "DateTime.Time(X)", "type time"),
    op("Time", "Hour", Time, "Time.Hour(X)", "Int64.Type"),
    op("Time", "Minute", Time, "Time.Minute(X)", "Int64.Type"),
    op("Time", "Second", Time, "Time.Second(X)", "Int64.Type"),
    op("Duration", "Days", Dur, "Duration.Days(X)", "Int64.Type"),
    op("Duration", "Hours", Dur, "Duration.Hours(X)", "Int64.Type"),
    op("Duration", "Minutes", Dur, "Duration.Minutes(X)", "Int64.Type"),
    op("Duration", "Seconds", Dur, "Duration.Seconds(X)", "Int64.Type"),
    op("Duration", "Total Years", Dur, "Duration.TotalDays(X) / 365", "type number"),
    op("Duration", "Total Days", Dur, "Duration.TotalDays(X)", "type number"),
    op("Duration", "Total Hours", Dur, "Duration.TotalHours(X)", "type number"),
    op("Duration", "Total Minutes", Dur, "Duration.TotalMinutes(X)", "type number"),
];

/// The selected columns, or a message asking for one.
pub fn need_columns(app: &mut VizApp) -> Option<Vec<String>> {
    let s = app.pq.selected.clone();
    if s.is_empty() {
        app.toast("Select a column first (click its header; Ctrl-click for more).");
        None
    } else {
        Some(s)
    }
}

fn first_column(app: &mut VizApp) -> Option<String> {
    need_columns(app).and_then(|c| c.first().cloned())
}

/// Applies a catalog operation: in place (`add == false`) or as a new column.
pub fn apply_op(app: &mut VizApp, i: usize, add: bool, param: Option<String>) {
    let Some(o) = OPS.get(i) else { return };
    let Some(cols) = need_columns(app) else { return };
    if o.param.is_some() && param.is_none() {
        app.pq.dialog = Some(Dialog::param(i, add));
        return;
    }
    let p = match (o.param, &param) {
        (Some((_, _, true)), Some(v)) => quote_text(v),
        (_, Some(v)) => v.clone(),
        _ => String::new(),
    };
    let body = o.f.replace('P', "\u{1}");
    let label = o.label.trim_end_matches('…');
    if add {
        let c = cols.first().cloned().unwrap_or_default();
        let e = body.replace('X', &field(&c)).replace('\u{1}', &p);
        add_step(app, &format!("Inserted {label}"), &format!("Table.AddColumn({{prev}}, {}, each {e}, {})", quote_text(label), o.ty));
    } else {
        let e = body.replace('X', "_").replace('\u{1}', &p);
        let specs: Vec<String> = cols.iter().map(|c| format!("{{{}, each {e}, {}}}", quote_text(c), o.ty)).collect();
        add_step(app, &step_name(o), &format!("Table.TransformColumns({{prev}}, {{{}}})", specs.join(", ")));
    }
}

fn step_name(o: &ColOp) -> String {
    match o.label {
        "lowercase" => "Lowercased Text".into(),
        "UPPERCASE" => "Uppercased Text".into(),
        "Capitalize Each Word" => "Capitalized Each Word".into(),
        "Trim" => "Trimmed Text".into(),
        "Clean" => "Cleaned Text".into(),
        "Add Prefix…" => "Added Prefix".into(),
        "Add Suffix…" => "Added Suffix".into(),
        l if matches!(o.menu, "Extract" | "Date" | "Time" | "Duration") => format!("Extracted {}", l.trim_end_matches('…')),
        "JSON" => "Parsed JSON".into(),
        "XML" => "Parsed XML".into(),
        l => format!("Calculated {}", l.trim_end_matches('…')),
    }
}

/// A ribbon dropdown: a tool whose menu holds `add`. Inside a menu it is an ordinary submenu.
pub fn ribbon_menu(ui: &mut Ui, t: &Tokens, label: &str, add: impl FnOnce(&mut Ui)) {
    if egui::containers::menu::is_in_menu(ui) {
        ui.menu_button(label, add);
        return;
    }
    let r = tool(ui, t, icon_for(label), label, menu_tip(label), true, false);
    egui::Popup::menu(&r).show(|ui| {
        ui.set_min_width(180.0);
        add(ui);
    });
}

/// A small dropdown (stacked in a column).
fn small_menu(ui: &mut Ui, t: &Tokens, label: &str, add: impl FnOnce(&mut Ui)) {
    if egui::containers::menu::is_in_menu(ui) {
        ui.menu_button(label, add);
        return;
    }
    let r = small_tool(ui, t, icon_for(label), label, menu_tip(label), true);
    egui::Popup::menu(&r).show(|ui| {
        ui.set_min_width(180.0);
        add(ui);
    });
}

fn big(ui: &mut Ui, t: &Tokens, label: &str, tip: &str) -> bool {
    tool(ui, t, icon_for(label), label, tip, false, false).clicked()
}

fn small(ui: &mut Ui, t: &Tokens, label: &str, tip: &str) -> bool {
    small_tool(ui, t, icon_for(label), label, tip, false).clicked()
}

/// A submenu of catalog operations of one menu (`Format`, `Extract`, `Date`, …).
pub fn op_menu(app: &mut VizApp, ui: &mut Ui, t: &Tokens, menu: &str, add: bool, smallish: bool) {
    let body = |ui: &mut Ui| {
        for (i, o) in OPS.iter().enumerate().filter(|(_, o)| o.menu == menu) {
            if ui.button(o.label).clicked() {
                apply_op(app, i, add, None);
                ui.close();
            }
        }
    };
    if smallish { small_menu(ui, t, menu, body) } else { ribbon_menu(ui, t, menu, body) }
}

pub fn change_type(app: &mut VizApp, cols: &[String], ty: MType) {
    let pairs: Vec<String> = cols.iter().map(|c| format!("{{{}, {}}}", quote_text(c), ty.m())).collect();
    add_step(app, "Changed Type", &format!("Table.TransformColumnTypes({{prev}}, {{{}}})", pairs.join(", ")));
}

fn type_items(app: &mut VizApp, ui: &mut Ui) {
    for ty in MType::COLUMN_TYPES {
        if ui.button(ty.name()).clicked() {
            if let Some(c) = need_columns(app) {
                change_type(app, &c, ty);
            }
            ui.close();
        }
    }
}

pub fn sort(app: &mut VizApp, desc: bool) {
    let Some(c) = need_columns(app) else { return };
    let keys: Vec<String> = c.iter().map(|x| format!("{{{}, Order.{}}}", quote_text(x), if desc { "Descending" } else { "Ascending" })).collect();
    add_step(app, "Sorted Rows", &format!("Table.Sort({{prev}}, {{{}}})", keys.join(", ")));
}

pub fn remove_columns(app: &mut VizApp, other: bool) {
    let Some(c) = need_columns(app) else { return };
    if other {
        add_step(app, "Removed Other Columns", &format!("Table.SelectColumns({{prev}}, {})", text_list(&c)));
    } else {
        add_step(app, "Removed Columns", &format!("Table.RemoveColumns({{prev}}, {})", text_list(&c)));
    }
    app.pq.selected.clear();
}

/// Moves the selected columns (Move Left/Right/To Beginning/To End).
pub fn move_columns(app: &mut VizApp, to: &str) {
    let Some(sel) = need_columns(app) else { return };
    let mut cols = preview_table(app).columns.clone();
    match to {
        "start" | "end" => {
            cols.retain(|c| !sel.contains(c));
            if to == "start" {
                let mut v = sel.clone();
                v.extend(cols);
                cols = v;
            } else {
                cols.extend(sel.iter().cloned());
            }
        }
        _ => {
            for s in &sel {
                if let Some(i) = cols.iter().position(|c| c == s) {
                    let j = if to == "left" { i.saturating_sub(1) } else { (i + 1).min(cols.len().saturating_sub(1)) };
                    cols.swap(i, j);
                }
            }
        }
    }
    add_step(app, "Reordered Columns", &format!("Table.ReorderColumns({{prev}}, {})", text_list(&cols)));
}

fn statistics(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    ribbon_menu(ui, t, "Statistics", |ui| {
        for (label, f) in [
            ("Sum", "List.Sum"),
            ("Minimum", "List.Min"),
            ("Maximum", "List.Max"),
            ("Median", "List.Median"),
            ("Average", "List.Average"),
            ("Standard Deviation", "List.StandardDeviation"),
            ("Count Values", "List.NonNullCount"),
            ("Count Distinct Values", "List.Distinct"),
        ] {
            if ui.button(label).clicked() {
                if let Some(c) = first_column(app) {
                    let e = if f == "List.Distinct" { format!("List.Count(List.Distinct({{prev}}{}))", field(&c)) } else { format!("{f}({{prev}}{})", field(&c)) };
                    add_step(app, label, &e);
                }
                ui.close();
            }
        }
    });
}

fn split_items(app: &mut VizApp, ui: &mut Ui) {
    for (label, mode) in [
        ("By Delimiter…", SplitMode::Delimiter),
        ("By Number of Characters…", SplitMode::Chars),
        ("By Positions…", SplitMode::Positions),
        ("By Lowercase to Uppercase", SplitMode::LowerUpper),
        ("By Uppercase to Lowercase", SplitMode::UpperLower),
        ("By Digit to Non-Digit", SplitMode::DigitNon),
        ("By Non-Digit to Digit", SplitMode::NonDigit),
    ] {
        if ui.button(label).clicked() {
            if let Some(c) = first_column(app) {
                let table = preview_table(app);
                app.pq.dialog = Some(Dialog::split(&c, mode, &table));
            }
            ui.close();
        }
    }
}

fn keep_rows_items(app: &mut VizApp, ui: &mut Ui) {
    for (label, k) in [("Keep Top Rows…", KeepRemove::KeepTop), ("Keep Bottom Rows…", KeepRemove::KeepBottom), ("Keep Range of Rows…", KeepRemove::KeepRange)] {
        if ui.button(label).clicked() {
            app.pq.dialog = Some(Dialog::rows(k));
            ui.close();
        }
    }
    ui.separator();
    if ui.button("Keep Duplicates").clicked() {
        if let Some(c) = need_columns(app) {
            add_step(
                app,
                "Kept Duplicates",
                &format!(
                    "let columnNames = {}, addCount = Table.Group({{prev}}, columnNames, {{{{\"Count\", Table.RowCount, type number}}}}), selectDuplicates = Table.SelectRows(addCount, each [Count] > 1), removeCount = Table.RemoveColumns(selectDuplicates, \"Count\") in Table.Join({{prev}}, columnNames, removeCount, columnNames, JoinKind.Inner)",
                    text_list(&c)
                ),
            );
        }
        ui.close();
    }
    if ui.button("Keep Errors").clicked() {
        let cols = app.pq.selected.clone();
        add_step(app, "Kept Errors", &if cols.is_empty() { "Table.SelectRowsWithErrors({prev})".to_string() } else { format!("Table.SelectRowsWithErrors({{prev}}, {})", text_list(&cols)) });
        ui.close();
    }
}

fn remove_rows_items(app: &mut VizApp, ui: &mut Ui) {
    for (label, k) in [("Remove Top Rows…", KeepRemove::RemoveTop), ("Remove Bottom Rows…", KeepRemove::RemoveBottom), ("Remove Alternate Rows…", KeepRemove::Alternate)] {
        if ui.button(label).clicked() {
            app.pq.dialog = Some(Dialog::rows(k));
            ui.close();
        }
    }
    ui.separator();
    if ui.button("Remove Duplicates").clicked() {
        let cols = app.pq.selected.clone();
        add_step(app, "Removed Duplicates", &if cols.is_empty() { "Table.Distinct({prev})".to_string() } else { format!("Table.Distinct({{prev}}, {})", text_list(&cols)) });
        ui.close();
    }
    if ui.button("Remove Blank Rows").clicked() {
        add_step(app, "Removed Blank Rows", "Table.SelectRows({prev}, each not List.IsEmpty(List.RemoveItems(Record.FieldValues(_), {\"\", null})))");
        ui.close();
    }
    if ui.button("Remove Errors").clicked() {
        let cols = app.pq.selected.clone();
        add_step(app, "Removed Errors", &if cols.is_empty() { "Table.RemoveRowsWithErrors({prev})".to_string() } else { format!("Table.RemoveRowsWithErrors({{prev}}, {})", text_list(&cols)) });
        ui.close();
    }
}

fn headers_items(app: &mut VizApp, ui: &mut Ui) {
    if ui.button("Use First Row as Headers").clicked() {
        add_step(app, "Promoted Headers", "Table.PromoteHeaders({prev}, [PromoteAllScalars=true])");
        ui.close();
    }
    if ui.button("Use Headers as First Row").clicked() {
        add_step(app, "Demoted Headers", "Table.DemoteHeaders({prev})");
        ui.close();
    }
}

fn unpivot_items(app: &mut VizApp, ui: &mut Ui) {
    if ui.button("Unpivot Columns").clicked() {
        if let Some(c) = need_columns(app) {
            let all = preview_table(app).columns.clone();
            let others: Vec<String> = all.into_iter().filter(|x| !c.contains(x)).collect();
            add_step(app, "Unpivoted Columns", &format!("Table.UnpivotOtherColumns({{prev}}, {}, \"Attribute\", \"Value\")", text_list(&others)));
        }
        ui.close();
    }
    if ui.button("Unpivot Other Columns").clicked() {
        if let Some(c) = need_columns(app) {
            add_step(app, "Unpivoted Other Columns", &format!("Table.UnpivotOtherColumns({{prev}}, {}, \"Attribute\", \"Value\")", text_list(&c)));
        }
        ui.close();
    }
    if ui.button("Unpivot Only Selected Columns").clicked() {
        if let Some(c) = need_columns(app) {
            add_step(app, "Unpivoted Only Selected Columns", &format!("Table.Unpivot({{prev}}, {}, \"Attribute\", \"Value\")", text_list(&c)));
        }
        ui.close();
    }
}

fn move_items(app: &mut VizApp, ui: &mut Ui) {
    for (label, to) in [("Left", "left"), ("Right", "right"), ("To Beginning", "start"), ("To End", "end")] {
        if ui.button(label).clicked() {
            move_columns(app, to);
            ui.close();
        }
    }
}

fn fill_items(app: &mut VizApp, ui: &mut Ui) {
    for (label, f) in [("Down", "FillDown"), ("Up", "FillUp")] {
        if ui.button(label).clicked() {
            if let Some(c) = need_columns(app) {
                add_step(app, &format!("Filled {label}"), &format!("Table.{f}({{prev}}, {})", text_list(&c)));
            }
            ui.close();
        }
    }
}

fn index_items(app: &mut VizApp, ui: &mut Ui) {
    if ui.button("From 0").clicked() {
        add_step(app, "Added Index", "Table.AddIndexColumn({prev}, \"Index\", 0, 1, Int64.Type)");
        ui.close();
    }
    if ui.button("From 1").clicked() {
        add_step(app, "Added Index", "Table.AddIndexColumn({prev}, \"Index\", 1, 1, Int64.Type)");
        ui.close();
    }
    if ui.button("Custom…").clicked() {
        app.pq.dialog = Some(Dialog::index());
        ui.close();
    }
}

/// Sets column types from the data (the selected columns, or all).
pub fn detect_types(app: &mut VizApp) {
    let table = preview_table(app);
    let cols: Vec<String> = if app.pq.selected.is_empty() { table.columns.clone() } else { app.pq.selected.clone() };
    let pairs = detected_types(&table, &cols);
    if pairs.is_empty() {
        app.toast("No types to detect.");
    } else {
        add_step(app, "Changed Type", &format!("Table.TransformColumnTypes({{prev}}, {{{}}})", pairs.join(", ")));
    }
}

/// `{"Col", type}` pairs for the columns whose values have one type.
pub fn detected_types(table: &vizcraft_query::Table, cols: &[String]) -> Vec<String> {
    let mut pairs = Vec::new();
    for c in cols {
        let Some(i) = table.column_index(c) else { continue };
        let vals: Vec<_> = table.rows.iter().take(200).map(|r| r.get(i).cloned().unwrap_or_default()).collect();
        let ty = vizcraft_query::funcs::convert::detect(&vals);
        if ty != MType::Any {
            pairs.push(format!("{{{}, {}}}", quote_text(c), ty.m()));
        }
    }
    pairs
}

/// The M that reads one file of a folder by its extension.
pub fn file_reader(ext: &str) -> &'static str {
    match ext {
        ".xlsx" | ".xlsm" | ".xlsb" | ".xls" | ".ods" => "Excel.Workbook([Content], true){0}[Data]",
        ".json" => "Table.FromRecords(Json.Document([Content]))",
        ".parquet" => "Parquet.Document([Content])",
        ".xml" => "Xml.Tables([Content]){0}[Table]",
        ".tsv" | ".tab" => "Table.PromoteHeaders(Csv.Document([Content], [Delimiter=\"#(tab)\", Encoding=65001, QuoteStyle=QuoteStyle.Csv]), [PromoteAllScalars=true])",
        _ => "Table.PromoteHeaders(Csv.Document([Content], [Encoding=65001, QuoteStyle=QuoteStyle.Csv]), [PromoteAllScalars=true])",
    }
}

/// Combine Files (folder queries): read every file the same way and append the results.
pub fn combine_files(app: &mut VizApp) {
    let t = preview_table(app);
    let (Some(_), Some(ext_c)) = (t.column_index("Content"), t.column_index("Extension")) else {
        app.toast("Combine Files works on a folder's file list (Content, Name, Extension columns).");
        return;
    };
    let ext = t.rows.first().and_then(|r| r.get(ext_c)).map(|v| v.display().to_lowercase()).unwrap_or_default();
    add_step(app, "Filtered Files", &format!("Table.SelectRows({{prev}}, each Text.Lower([Extension]) = {})", quote_text(&ext)));
    add_step(app, "Transform File", &format!("Table.AddColumn({{prev}}, \"Transform File\", each {})", file_reader(&ext)));
    add_step(app, "Removed Other Columns", "Table.SelectColumns({prev}, {\"Name\", \"Transform File\"})");
    let p = preview_table(app);
    let first = p.column_index("Transform File").and_then(|c| p.rows.first().and_then(|r| r.get(c)).cloned());
    match first {
        Some(vizcraft_query::Value::Table(nt)) => {
            add_step(app, "Expanded Table Column", &format!("Table.ExpandTableColumn({{prev}}, \"Transform File\", {})", text_list(&nt.columns)));
            app.pq.selected.clear();
            detect_types(app);
        }
        _ => app.toast("The first file couldn't be read; check the Transform File step."),
    }
}

/// Hover descriptions of the ribbon's dropdowns.
fn menu_tip(label: &str) -> &'static str {
    match label {
        "New Source" => "Get data from a file, a folder, a database, the web, or type it in",
        "Recent Sources" => "Connect again to a source used recently",
        "Manage" => "Duplicate, reference or delete this query",
        "Remove Columns" => "Remove the selected columns, or every other column",
        "Keep Rows" => "Keep the top, bottom or a range of rows, duplicates or errors",
        "Remove Rows" => "Remove top, bottom, alternate, duplicate, blank or error rows",
        "Split Column" => "Split the selected column by a delimiter, a number of characters, positions or case transitions",
        "Data Type" => "Set the type of the selected columns",
        "Use First Row as Headers" => "Promote the first row to headers, or demote the headers to a row",
        "Replace Values" => "Replace values or errors in the selected columns",
        "Merge Queries" => "Join another query's columns by matching key columns",
        "Append Queries" => "Add the rows of other queries under this one",
        "Manage Parameters" => "Values queries can refer to by name",
        "Fill" => "Fill empty cells down or up from their neighbours",
        "Unpivot Columns" => "Turn columns into attribute / value rows",
        "Move" => "Move the selected columns left, right, to the beginning or the end",
        "Statistics" => "Sum, minimum, maximum, average, count … of the selected column",
        "Format" => "Lowercase, UPPERCASE, Capitalize, Trim, Clean, add a prefix or suffix",
        "Extract" => "Length, first or last characters, a range, or text between delimiters",
        "Parse" => "Read JSON or XML text into structured values",
        "Standard" => "Add, subtract, multiply, divide, modulo, percentage …",
        "Scientific" => "Absolute value, power, square root, exponent, logarithms, factorial",
        "Trigonometry" => "Sine, cosine, tangent and their inverses",
        "Rounding" => "Round up, round down or round to a number of digits",
        "Information" => "Is even, is odd, sign",
        "Date" => "Year, month, quarter, week, day, age, start or end of a period …",
        "Time" => "Hour, minute, second, start of hour",
        "Duration" => "Days, hours, minutes, seconds, total years …",
        "Column From Examples" => "Type a few example results; the transformation is worked out for you",
        "Index Column" => "A row number column, from 0, from 1 or custom",
        "Go to Column" => "Select a column by name",
        "Structured Column" => "Expand or aggregate a column of tables, records or lists",
        _ => "",
    }
}

/// The ribbon icon of a command.
fn icon_for(label: &str) -> &'static str {
    match label {
        "New Source" => "new-source",
        "Recent Sources" => "recent",
        "Enter Data" => "enter-data",
        "Refresh Preview" => "refresh",
        "Refresh All" => "refresh-all",
        "Advanced Editor" => "advanced-editor",
        "Manage" => "manage",
        "Choose Columns" => "choose-columns",
        "Remove Columns" => "remove-columns",
        "Keep Rows" => "keep-rows",
        "Remove Rows" => "remove-rows",
        "Sort Ascending" => "sort-asc",
        "Sort Descending" => "sort-desc",
        "Split Column" => "split-column",
        "Group By" => "group-by",
        "Data Type" => "data-type",
        "Use First Row as Headers" => "headers",
        "Replace Values" => "replace",
        "Merge Queries" => "merge-queries",
        "Append Queries" => "append-queries",
        "Combine Files" => "combine-files",
        "Manage Parameters" => "parameters",
        "Close &\nApply" => "close-apply",
        "Close" => "close",
        "Transpose" => "transpose",
        "Reverse Rows" => "reverse-rows",
        "Count Rows" => "count-rows",
        "Detect Data Type" => "detect-type",
        "Rename" => "rename",
        "Fill" => "fill",
        "Pivot Column" => "pivot-column",
        "Unpivot Columns" => "unpivot",
        "Move" => "move-columns",
        "Convert to List" => "to-list",
        "Format" => "format",
        "Merge Columns" => "merge-columns",
        "Extract" => "extract",
        "Parse" => "parse",
        "Statistics" => "statistics",
        "Standard" => "standard",
        "Scientific" => "scientific",
        "Trigonometry" => "scientific",
        "Rounding" => "rounding",
        "Information" => "information",
        "Date" => "date",
        "Time" => "time",
        "Duration" => "duration",
        "Structured Column" => "expand-column",
        "Expand" => "expand-column",
        "Aggregate" => "statistics",
        "Column From Examples" => "from-examples",
        "Custom Column" => "custom-column",
        "Invoke Custom Function" => "invoke-function",
        "Conditional Column" => "conditional",
        "Index Column" => "index-column",
        "Duplicate Column" => "duplicate-column",
        "Query Dependencies" => "dependencies",
        "Go to Column" => "go-to-column",
        "Run Python script" => "python",
        "Formula Bar" => "advanced-editor",
        "Monospaced" => "monospace",
        "Show Whitespace" => "whitespace",
        "Column Quality" => "quality",
        "Column Distribution" => "profile",
        "Column Profile" => "profile",
        "Profile Entire Data Set" => "all-rows",
        "Queries Pane" => "query",
        "Query Settings" => "settings",
        "Data Source Canvas" => "data-source",
        "Diagnose Step" => "refresh",
        "M Reference" => "help",
        "Keyboard Shortcuts" => "help",
        "About" => "help",
        _ => "query",
    }
}

/// The editor's ribbon for the active tab.
pub fn ribbon(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    egui::ScrollArea::horizontal().id_salt("pq_ribbon_scroll").show(ui, |ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(1.0, 0.0);
            match app.pq.tab {
                0 => home(app, ui, t),
                1 => transform(app, ui, t),
                2 => add_column(app, ui, t),
                3 => view(app, ui, t),
                4 => tools(app, ui, t),
                _ => help(app, ui, t),
            }
        });
    });
}

fn home(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    group(ui, t, "Close", |ui| {
        if tool(ui, t, "close-apply", "Close &\nApply", "Load the queries into the model, refresh and close the editor", false, false).clicked() {
            super::close_and_apply(app);
        }
        if big(ui, t, "Close", "Close the editor (Esc). Edits stay in the queries; apply them later.") {
            app.pq_open = false;
            if app.pq.pending {
                app.toast("Query changes aren't applied yet: use Close & Apply or Refresh to load them.");
            }
        }
    });
    group(ui, t, "New Query", |ui| {
        ribbon_menu(ui, t, "New Source", |ui| new_source_items(app, ui));
        ribbon_menu(ui, t, "Recent Sources", |ui| recent_items(app, ui));
        if big(ui, t, "Enter Data", "Type or paste a small table") {
            super::get_data(app, "enter");
        }
    });
    group(ui, t, "Parameters", |ui| {
        ribbon_menu(ui, t, "Manage Parameters", |ui| {
            if ui.button("Manage Parameters").clicked() {
                app.pq.dialog = Some(Dialog::parameters(app, None));
                ui.close();
            }
            if ui.button("New Parameter").clicked() {
                app.pq.dialog = Some(Dialog::parameters(app, Some(String::new())));
                ui.close();
            }
        });
    });
    group(ui, t, "Query", |ui| {
        if big(ui, t, "Refresh Preview", "Re-evaluate the preview (F5)") {
            app.pq.refresh_preview();
        }
        stack(ui, |ui| {
            if small_tool(ui, t, "settings", "Properties", "Name, description and group of the query", false).clicked() {
                app.pq.dialog = Some(Dialog::properties(app));
            }
            if small(ui, t, "Advanced Editor", "Edit the whole query in M") {
                app.pq.dialog = Some(Dialog::advanced(app));
            }
            small_menu(ui, t, "Manage", |ui| manage_items(app, ui));
        });
    });
    group(ui, t, "Manage Columns", |ui| {
        if big(ui, t, "Choose Columns", "Pick the columns to keep") {
            app.pq.dialog = Some(Dialog::choose_columns(app));
        }
        ribbon_menu(ui, t, "Remove Columns", |ui| {
            if ui.button("Remove Columns").clicked() {
                remove_columns(app, false);
                ui.close();
            }
            if ui.button("Remove Other Columns").clicked() {
                remove_columns(app, true);
                ui.close();
            }
        });
    });
    group(ui, t, "Reduce Rows", |ui| {
        ribbon_menu(ui, t, "Keep Rows", |ui| keep_rows_items(app, ui));
        ribbon_menu(ui, t, "Remove Rows", |ui| remove_rows_items(app, ui));
    });
    group(ui, t, "Sort", |ui| {
        stack(ui, |ui| {
            if small(ui, t, "Sort Ascending", "Sort ascending by the selected columns") {
                sort(app, false);
            }
            if small(ui, t, "Sort Descending", "Sort descending by the selected columns") {
                sort(app, true);
            }
        });
    });
    group(ui, t, "Transform", |ui| {
        ribbon_menu(ui, t, "Split Column", |ui| split_items(app, ui));
        if big(ui, t, "Group By", "Summarize rows by the values of columns") {
            app.pq.dialog = Some(Dialog::group_by(app));
        }
        stack(ui, |ui| {
            small_menu(ui, t, "Data Type", |ui| type_items(app, ui));
            small_menu(ui, t, "Use First Row as Headers", |ui| headers_items(app, ui));
            if small(ui, t, "Replace Values", "Replace a value in the selected columns")
                && let Some(c) = need_columns(app)
            {
                app.pq.dialog = Some(Dialog::replace(c));
            }
        });
    });
    group(ui, t, "Combine", |ui| {
        stack(ui, |ui| {
            small_menu(ui, t, "Merge Queries", |ui| {
                if ui.button("Merge Queries").clicked() {
                    app.pq.dialog = Some(Dialog::merge(app, false));
                    ui.close();
                }
                if ui.button("Merge Queries as New").clicked() {
                    app.pq.dialog = Some(Dialog::merge(app, true));
                    ui.close();
                }
            });
            small_menu(ui, t, "Append Queries", |ui| {
                if ui.button("Append Queries").clicked() {
                    app.pq.dialog = Some(Dialog::append(false));
                    ui.close();
                }
                if ui.button("Append Queries as New").clicked() {
                    app.pq.dialog = Some(Dialog::append(true));
                    ui.close();
                }
            });
            if small(ui, t, "Combine Files", "Read every file of a folder query and append them") {
                combine_files(app);
            }
        });
    });
}

/// The New Source / Get Data items.
pub fn new_source_items(app: &mut VizApp, ui: &mut Ui) {
    for (kind, label) in [
        ("csv", "Text/CSV"),
        ("excel", "Excel workbook"),
        ("json", "JSON"),
        ("xml", "XML"),
        ("parquet", "Parquet"),
        ("folder", "Folder"),
        ("web", "Web"),
        ("odata", "OData feed"),
        ("sqlite", "SQLite database"),
        ("postgresql", "PostgreSQL database"),
        ("mysql", "MySQL database"),
        ("sqlserver", "SQL Server database"),
        ("enter", "Enter data"),
        ("blank", "Blank query"),
        ("more", "More…"),
    ] {
        if ui.button(label).clicked() {
            super::get_data(app, kind);
            ui.close();
        }
    }
}

fn recent_items(app: &mut VizApp, ui: &mut Ui) {
    if app.pq.recent.is_empty() {
        ui.label(egui::RichText::new("No recent sources yet").italics());
    }
    for r in app.pq.recent.clone() {
        let label = format!("{}  ·  {}", super::getdata::kind_label(&r.kind), r.target);
        if ui.button(label).clicked() {
            super::getdata::open_recent(app, &r);
            ui.close();
        }
    }
}

fn manage_items(app: &mut VizApp, ui: &mut Ui) {
    let q = app.pq.query.clone();
    if ui.button("Duplicate").clicked() {
        if let Some(n) = super::duplicate_query(app, &q) {
            open_editor(app, Some(&n));
        }
        ui.close();
    }
    if ui.button("Reference").clicked() {
        if let Some(n) = super::reference_query(app, &q) {
            open_editor(app, Some(&n));
        }
        ui.close();
    }
    if ui.button("Delete").clicked() {
        super::delete_query(app, &q);
        ui.close();
    }
}

fn transform(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    group(ui, t, "Table", |ui| {
        if big(ui, t, "Group By", "Summarize rows by the values of columns") {
            app.pq.dialog = Some(Dialog::group_by(app));
        }
        ribbon_menu(ui, t, "Use First Row as Headers", |ui| headers_items(app, ui));
        stack(ui, |ui| {
            if small(ui, t, "Transpose", "Rows become columns") {
                add_step(app, "Transposed Table", "Table.Transpose({prev})");
            }
            if small(ui, t, "Reverse Rows", "Last row first") {
                add_step(app, "Reversed Rows", "Table.ReverseRows({prev})");
            }
            if small(ui, t, "Count Rows", "The number of rows") {
                add_step(app, "Counted Rows", "Table.RowCount({prev})");
            }
        });
    });
    group(ui, t, "Any Column", |ui| {
        stack(ui, |ui| {
            small_menu(ui, t, "Data Type", |ui| type_items(app, ui));
            if small(ui, t, "Detect Data Type", "Set types from the data") {
                detect_types(app);
            }
            if small(ui, t, "Rename", "Rename the selected column")
                && let Some(c) = first_column(app)
            {
                app.pq.dialog = Some(Dialog::rename_column(&c));
            }
        });
        stack(ui, |ui| {
            small_menu(ui, t, "Replace Values", |ui| {
                if ui.button("Replace Values").clicked() {
                    if let Some(c) = need_columns(app) {
                        app.pq.dialog = Some(Dialog::replace(c));
                    }
                    ui.close();
                }
                if ui.button("Replace Errors").clicked() {
                    if let Some(c) = need_columns(app) {
                        app.pq.dialog = Some(Dialog::replace_errors(c));
                    }
                    ui.close();
                }
            });
            small_menu(ui, t, "Fill", |ui| fill_items(app, ui));
            if small(ui, t, "Pivot Column", "Turn the values of the selected column into columns")
                && let Some(c) = first_column(app)
            {
                app.pq.dialog = Some(Dialog::pivot(app, &c));
            }
        });
        stack(ui, |ui| {
            small_menu(ui, t, "Unpivot Columns", |ui| unpivot_items(app, ui));
            small_menu(ui, t, "Move", |ui| move_items(app, ui));
            if small(ui, t, "Convert to List", "The selected column as a list")
                && let Some(c) = first_column(app)
            {
                add_step(app, &c, &format!("{{prev}}{}", field(&c)));
            }
        });
    });
    group(ui, t, "Text Column", |ui| {
        ribbon_menu(ui, t, "Split Column", |ui| split_items(app, ui));
        op_menu(app, ui, t, "Format", false, false);
        stack(ui, |ui| {
            if small(ui, t, "Merge Columns", "Join the selected columns' text")
                && let Some(c) = need_columns(app)
            {
                app.pq.dialog = Some(Dialog::merge_columns(c, false));
            }
            op_menu(app, ui, t, "Extract", false, true);
            op_menu(app, ui, t, "Parse", false, true);
        });
    });
    group(ui, t, "Number Column", |ui| {
        statistics(app, ui, t);
        op_menu(app, ui, t, "Standard", false, false);
        op_menu(app, ui, t, "Scientific", false, false);
        stack(ui, |ui| {
            op_menu(app, ui, t, "Trigonometry", false, true);
            op_menu(app, ui, t, "Rounding", false, true);
            op_menu(app, ui, t, "Information", false, true);
        });
    });
    group(ui, t, "Date & Time Column", |ui| {
        op_menu(app, ui, t, "Date", false, false);
        op_menu(app, ui, t, "Time", false, false);
        op_menu(app, ui, t, "Duration", false, false);
    });
    group(ui, t, "Structured Column", |ui| {
        stack(ui, |ui| {
            if small(ui, t, "Expand", "Expand the selected table, record or list column")
                && let Some(c) = first_column(app)
            {
                app.pq.dialog = Some(Dialog::expand(app, &c, false));
            }
            if small(ui, t, "Aggregate", "Summarize the tables in the selected column")
                && let Some(c) = first_column(app)
            {
                app.pq.dialog = Some(Dialog::expand(app, &c, true));
            }
        });
    });
    group(ui, t, "Scripts", |ui| {
        if big(ui, t, "Run Python script", "Transform the table with a Python script (pandas DataFrame `dataset`)") {
            app.pq.dialog = Some(Dialog::python());
        }
    });
}

fn add_column(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    group(ui, t, "General", |ui| {
        ribbon_menu(ui, t, "Column From Examples", |ui| {
            if ui.button("From All Columns").clicked() {
                app.pq.examples = Some(super::Examples { name: "Custom".into(), ..Default::default() });
                ui.close();
            }
            if ui.button("From Selection").clicked() {
                if let Some(c) = need_columns(app) {
                    app.pq.examples = Some(super::Examples { name: "Custom".into(), inputs: c, ..Default::default() });
                }
                ui.close();
            }
        });
        if big(ui, t, "Custom Column", "A new column computed by an M formula") {
            app.pq.dialog = Some(Dialog::custom_column(app));
        }
        if big(ui, t, "Invoke Custom Function", "Call a function query for every row") {
            app.pq.dialog = Some(Dialog::invoke(app));
        }
        stack(ui, |ui| {
            if small(ui, t, "Conditional Column", "If … then … else rules") {
                app.pq.dialog = Some(Dialog::conditional(app));
            }
            small_menu(ui, t, "Index Column", |ui| index_items(app, ui));
            if small(ui, t, "Duplicate Column", "Copy the selected column")
                && let Some(c) = first_column(app)
            {
                add_step(app, "Duplicated Column", &format!("Table.DuplicateColumn({{prev}}, {}, {})", quote_text(&c), quote_text(&format!("{c} - Copy"))));
            }
        });
    });
    group(ui, t, "From Text", |ui| {
        op_menu(app, ui, t, "Format", true, false);
        stack(ui, |ui| {
            if small(ui, t, "Merge Columns", "A new column joining the selected columns' text")
                && let Some(c) = need_columns(app)
            {
                app.pq.dialog = Some(Dialog::merge_columns(c, true));
            }
            op_menu(app, ui, t, "Extract", true, true);
            op_menu(app, ui, t, "Parse", true, true);
        });
    });
    group(ui, t, "From Number", |ui| {
        op_menu(app, ui, t, "Standard", true, false);
        op_menu(app, ui, t, "Scientific", true, false);
        stack(ui, |ui| {
            op_menu(app, ui, t, "Trigonometry", true, true);
            op_menu(app, ui, t, "Rounding", true, true);
            op_menu(app, ui, t, "Information", true, true);
        });
    });
    group(ui, t, "From Date & Time", |ui| {
        op_menu(app, ui, t, "Date", true, false);
        op_menu(app, ui, t, "Time", true, false);
        op_menu(app, ui, t, "Duration", true, false);
    });
}

fn view(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    let toggle = |ui: &mut Ui, label: &str, tip: &str, on: &mut bool| {
        if tool(ui, t, icon_for(label), label, tip, false, *on).clicked() {
            *on = !*on;
        }
    };
    group(ui, t, "Layout", |ui| {
        let v = &mut app.pq.view;
        toggle(ui, "Queries Pane", "Show the list of queries on the left", &mut v.queries_pane);
        toggle(ui, "Query Settings", "Show the properties and applied steps on the right", &mut v.settings_pane);
        toggle(ui, "Formula Bar", "Show the selected step's M formula above the preview", &mut v.formula_bar);
    });
    group(ui, t, "Data Preview", |ui| {
        let v = &mut app.pq.view;
        toggle(ui, "Monospaced", "Show the preview in a fixed-width font", &mut v.monospace);
        toggle(ui, "Show Whitespace", "Mark spaces and line breaks in the preview", &mut v.whitespace);
        toggle(ui, "Column Quality", "Valid, error and empty percentages under each header", &mut v.quality);
        toggle(ui, "Column Distribution", "Distinct and unique counts with a small histogram under each header", &mut v.distribution);
        toggle(ui, "Column Profile", "Statistics and value distribution of the selected column", &mut v.profile);
        let mut all = v.all_rows;
        toggle(ui, "Profile Entire Data Set", "Off: column profiling uses the top 1000 rows", &mut all);
        if all != app.pq.view.all_rows {
            app.pq.view.all_rows = all;
            app.pq.profiles.clear();
        }
    });
    group(ui, t, "Columns", |ui| {
        ribbon_menu(ui, t, "Go to Column", |ui| {
            let cols = preview_table(app).columns.clone();
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for c in cols {
                    if ui.button(&c).clicked() {
                        app.pq.selected = vec![c];
                        ui.close();
                    }
                }
            });
        });
    });
    group(ui, t, "Advanced", |ui| {
        if big(ui, t, "Advanced Editor", "Edit the whole query in M") {
            app.pq.dialog = Some(Dialog::advanced(app));
        }
    });
    group(ui, t, "Dependencies", |ui| {
        if big(ui, t, "Query Dependencies", "How sources, queries and the model connect") {
            app.pq.deps = true;
        }
    });
}

fn tools(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    group(ui, t, "Combine", |ui| {
        if big(ui, t, "Data Source Canvas", "Drag tables onto a canvas to join and union them (Tableau style)") {
            app.pq.page = super::Page::Canvas;
        }
    });
    group(ui, t, "Diagnostics", |ui| {
        if big(ui, t, "Diagnose Step", "Evaluate the selected step again and report how long it took") {
            app.pq.refresh_preview();
            if let Some(p) = super::preview(app) {
                let rows = p.table.rows.len();
                app.toast(format!("Step evaluated in {:.1} ms · {} rows · {} columns", p.ms, rows, p.table.columns.len()));
            }
        }
        if big(ui, t, "Refresh All", "Apply the queries to the model and refresh every table") {
            super::apply_to_model(app);
        }
    });
}

fn help(app: &mut VizApp, ui: &mut Ui, t: &Tokens) {
    group(ui, t, "Help", |ui| {
        if big(ui, t, "M Reference", "Every M function VizCraft knows, searchable") {
            app.pq.dialog = Some(Dialog::reference());
        }
        if big(ui, t, "Keyboard Shortcuts", "Keys of the Power Query Editor") {
            app.pq.dialog = Some(Dialog::shortcuts());
        }
    });
}

/// The menu of a column header (right click, or the header's dropdown).
pub fn column_menu(app: &mut VizApp, ui: &mut Ui, col: &str) {
    if !app.pq.selected.iter().any(|c| c == col) {
        app.pq.selected = vec![col.to_string()];
    }
    if ui.button("Remove").clicked() {
        remove_columns(app, false);
        ui.close();
    }
    if ui.button("Remove Other Columns").clicked() {
        remove_columns(app, true);
        ui.close();
    }
    if ui.button("Duplicate Column").clicked() {
        add_step(app, "Duplicated Column", &format!("Table.DuplicateColumn({{prev}}, {}, {})", quote_text(col), quote_text(&format!("{col} - Copy"))));
        ui.close();
    }
    if ui.button("Add Column From Examples…").clicked() {
        app.pq.examples = Some(super::Examples { name: "Custom".into(), inputs: app.pq.selected.clone(), ..Default::default() });
        ui.close();
    }
    ui.separator();
    if ui.button("Remove Duplicates").clicked() {
        let c = app.pq.selected.clone();
        add_step(app, "Removed Duplicates", &format!("Table.Distinct({{prev}}, {})", text_list(&c)));
        ui.close();
    }
    if ui.button("Remove Errors").clicked() {
        let c = app.pq.selected.clone();
        add_step(app, "Removed Errors", &format!("Table.RemoveRowsWithErrors({{prev}}, {})", text_list(&c)));
        ui.close();
    }
    ui.separator();
    ui.menu_button("Change Type", |ui| type_items(app, ui));
    ui.menu_button("Transform", |ui| {
        for m in ["Format", "Extract", "Parse", "Standard", "Rounding", "Date", "Time"] {
            ui.menu_button(m, |ui| {
                for (i, o) in OPS.iter().enumerate().filter(|(_, o)| o.menu == m) {
                    if ui.button(o.label).clicked() {
                        apply_op(app, i, false, None);
                        ui.close();
                    }
                }
            });
        }
    });
    ui.separator();
    if ui.button("Replace Values…").clicked() {
        app.pq.dialog = Some(Dialog::replace(app.pq.selected.clone()));
        ui.close();
    }
    if ui.button("Replace Errors…").clicked() {
        app.pq.dialog = Some(Dialog::replace_errors(app.pq.selected.clone()));
        ui.close();
    }
    ui.menu_button("Split Column", |ui| split_items(app, ui));
    if ui.button("Group By…").clicked() {
        app.pq.dialog = Some(Dialog::group_by(app));
        ui.close();
    }
    ui.menu_button("Fill", |ui| fill_items(app, ui));
    ui.menu_button("Unpivot", |ui| unpivot_items(app, ui));
    if ui.button("Rename…").clicked() {
        app.pq.dialog = Some(Dialog::rename_column(col));
        ui.close();
    }
    ui.menu_button("Move", |ui| move_items(app, ui));
    if ui.button("Drill Down").clicked() {
        add_step(app, col, &format!("{{prev}}{}", field(col)));
        ui.close();
    }
}
