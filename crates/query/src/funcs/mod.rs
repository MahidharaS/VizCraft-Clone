//! The M function library. [`call`] dispatches a library function by name; [`exists`] and
//! [`NAMES`] list them (the editor offers them for autocompletion).

pub mod convert;
mod date;
mod list;
mod number;
mod other;
pub mod html;
pub mod source;
pub mod table;
mod text;

use std::rc::Rc;

use crate::m::eval::{Evaluator, Function, check};
use crate::value::{MType, Record, Table, Value};
use crate::{Error, Result};

pub fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Eval(msg.into()))
}

/// The argument at `i` (null when missing).
pub fn arg(args: &[Value], i: usize) -> &Value {
    args.get(i).unwrap_or_else(|| crate::value::null_ref())
}

pub fn table_arg(args: &[Value], i: usize, f: &str) -> Result<Rc<Table>> {
    match check(arg(args, i).clone())? {
        Value::Table(t) => Ok(t),
        other => err(format!("{f}: expected a table, got {}.", other.kind())),
    }
}

pub fn list_arg(args: &[Value], i: usize, f: &str) -> Result<Rc<Vec<Value>>> {
    match check(arg(args, i).clone())? {
        Value::List(l) => Ok(l),
        other => err(format!("{f}: expected a list, got {}.", other.kind())),
    }
}

pub fn text_arg(args: &[Value], i: usize, f: &str) -> Result<String> {
    match check(arg(args, i).clone())? {
        Value::Text(t) => Ok(t.to_string()),
        other => err(format!("{f}: expected text, got {}.", other.kind())),
    }
}

pub fn opt_text(args: &[Value], i: usize) -> Option<String> {
    arg(args, i).as_text().map(str::to_string)
}

pub fn num_arg(args: &[Value], i: usize, f: &str) -> Result<f64> {
    match check(arg(args, i).clone())? {
        Value::Number(n) => Ok(n),
        other => err(format!("{f}: expected a number, got {}.", other.kind())),
    }
}

pub fn opt_num(args: &[Value], i: usize) -> Option<f64> {
    arg(args, i).as_number()
}

pub fn count_arg(args: &[Value], i: usize, f: &str) -> Result<usize> {
    let n = num_arg(args, i, f)?;
    if !(0.0..=1e9).contains(&n) {
        return err(format!("{f}: the count {n} is out of range."));
    }
    Ok(n as usize)
}

/// Column names given as one name or a list of names.
pub fn names_arg(v: &Value) -> Result<Vec<String>> {
    match v {
        Value::Text(t) => Ok(vec![t.to_string()]),
        Value::List(l) => l.iter().map(|x| x.as_text().map(str::to_string).ok_or_else(|| Error::Eval("expected column names".into()))).collect(),
        Value::Null => Ok(vec![]),
        other => err(format!("expected column names, got {}", other.kind())),
    }
}

pub fn func_arg(args: &[Value], i: usize, f: &str) -> Result<Value> {
    match arg(args, i) {
        v @ Value::Function(_) => Ok(v.clone()),
        other => err(format!("{f}: expected a function, got {}.", other.kind())),
    }
}

pub fn native(f: impl Fn(&Evaluator, Vec<Value>) -> Result<Value> + 'static) -> Value {
    Value::Function(Rc::new(Function::Native(Rc::new(f))))
}

/// Named constants: enumerations (`Order.Descending`, `JoinKind.LeftOuter`) and types.
pub fn constant(name: &str) -> Option<Value> {
    let n = |x: f64| Some(Value::Number(x));
    match name {
        "Order.Ascending" => n(0.0),
        "Order.Descending" => n(1.0),
        "JoinKind.Inner" => n(0.0),
        "JoinKind.LeftOuter" => n(1.0),
        "JoinKind.RightOuter" => n(2.0),
        "JoinKind.FullOuter" => n(3.0),
        "JoinKind.LeftAnti" => n(4.0),
        "JoinKind.RightAnti" => n(5.0),
        "MissingField.Error" => n(0.0),
        "MissingField.Ignore" => n(1.0),
        "MissingField.UseNull" => n(2.0),
        "QuoteStyle.None" => n(0.0),
        "QuoteStyle.Csv" => n(1.0),
        "ExtraValues.List" => n(0.0),
        "ExtraValues.Error" => n(1.0),
        "ExtraValues.Ignore" => n(2.0),
        "RelativePosition.FromStart" => n(0.0),
        "RelativePosition.FromEnd" => n(1.0),
        "Occurrence.First" => n(0.0),
        "Occurrence.Last" => n(1.0),
        "Occurrence.All" => n(2.0),
        "RoundingMode.Up" => n(0.0),
        "RoundingMode.Down" => n(1.0),
        "RoundingMode.AwayFromZero" => n(2.0),
        "RoundingMode.TowardZero" => n(3.0),
        "RoundingMode.ToEven" => n(4.0),
        "GroupKind.Global" => n(1.0),
        "GroupKind.Local" => n(0.0),
        "Day.Sunday" => n(0.0),
        "Day.Monday" => n(1.0),
        "Day.Tuesday" => n(2.0),
        "Day.Wednesday" => n(3.0),
        "Day.Thursday" => n(4.0),
        "Day.Friday" => n(5.0),
        "Day.Saturday" => n(6.0),
        "TextEncoding.Utf8" => n(65001.0),
        "TextEncoding.Utf16" | "TextEncoding.Unicode" => n(1200.0),
        "TextEncoding.Windows" => n(1252.0),
        "TextEncoding.Ascii" => n(20127.0),
        "Csv.Type" | "Any.Type" => Some(Value::Type(MType::Any)),
        "Int64.Type" | "Int32.Type" | "Int16.Type" | "Int8.Type" => Some(Value::Type(MType::Int64)),
        "Number.Type" | "Double.Type" | "Decimal.Type" | "Single.Type" => Some(Value::Type(MType::Number)),
        "Currency.Type" => Some(Value::Type(MType::Currency)),
        "Percentage.Type" => Some(Value::Type(MType::Percentage)),
        "Text.Type" => Some(Value::Type(MType::Text)),
        "Date.Type" => Some(Value::Type(MType::Date)),
        "DateTime.Type" | "DateTimeZone.Type" => Some(Value::Type(MType::DateTime)),
        "Time.Type" => Some(Value::Type(MType::Time)),
        "Duration.Type" => Some(Value::Type(MType::Duration)),
        "Logical.Type" => Some(Value::Type(MType::Logical)),
        "Table.Type" => Some(Value::Type(MType::Table)),
        "List.Type" => Some(Value::Type(MType::List)),
        "Record.Type" => Some(Value::Type(MType::Record)),
        "Binary.Type" => Some(Value::Type(MType::Binary)),
        "Culture.Current" => Some(Value::text("en-US")),
        _ => None,
    }
}

pub fn exists(name: &str) -> bool {
    NAMES.binary_search(&name).is_ok()
}

pub fn call(ev: &Evaluator, name: &str, args: Vec<Value>) -> Result<Value> {
    ev.tick()?;
    let r = table::call(ev, name, &args)
        .or_else(|| text::call(ev, name, &args))
        .or_else(|| list::call(ev, name, &args))
        .or_else(|| number::call(ev, name, &args))
        .or_else(|| date::call(ev, name, &args))
        .or_else(|| source::call(ev, name, &args))
        .or_else(|| other::call(ev, name, &args));
    match r {
        Some(r) => r.map_err(|e| match e {
            Error::Eval(m) if !m.contains(name) && !m.starts_with("DataFormat") && !m.starts_with("Expression") => {
                Error::Eval(format!("{name}: {m}"))
            }
            e => e,
        }),
        None => err(format!("The function {name} isn't supported yet.")),
    }
}

/// Builds a record from pairs.
pub fn rec(pairs: Vec<(&str, Value)>) -> Value {
    Value::record(Record::from_pairs(pairs.into_iter().map(|(n, v)| (n.to_string(), v)).collect()))
}

/// Every library function, sorted (for lookups and autocomplete).
pub const NAMES: &[&str] = &[
    "#date",
    "#datetime",
    "#duration",
    "#table",
    "#time",
    "Binary.FromText",
    "Binary.ToText",
    "Character.FromNumber",
    "Character.ToNumber",
    "Combiner.CombineTextByDelimiter",
    "Combiner.CombineTextByLengths",
    "Comparer.Ordinal",
    "Comparer.OrdinalIgnoreCase",
    "Csv.Document",
    "Currency.From",
    "Date.AddDays",
    "Date.AddMonths",
    "Date.AddQuarters",
    "Date.AddWeeks",
    "Date.AddYears",
    "Date.Day",
    "Date.DayOfWeek",
    "Date.DayOfWeekName",
    "Date.DayOfYear",
    "Date.DaysInMonth",
    "Date.EndOfMonth",
    "Date.EndOfQuarter",
    "Date.EndOfWeek",
    "Date.EndOfYear",
    "Date.From",
    "Date.FromText",
    "Date.IsLeapYear",
    "Date.Month",
    "Date.MonthName",
    "Date.QuarterOfYear",
    "Date.StartOfMonth",
    "Date.StartOfQuarter",
    "Date.StartOfWeek",
    "Date.StartOfYear",
    "Date.ToText",
    "Date.WeekOfMonth",
    "Date.WeekOfYear",
    "Date.Year",
    "DateTime.AddZone",
    "DateTime.Date",
    "DateTime.From",
    "DateTime.FromText",
    "DateTime.LocalNow",
    "DateTime.Time",
    "DateTime.ToText",
    "Decimal.From",
    "Double.From",
    "Duration.Days",
    "Duration.From",
    "Duration.FromText",
    "Duration.Hours",
    "Duration.Minutes",
    "Duration.Seconds",
    "Duration.ToText",
    "Duration.TotalDays",
    "Duration.TotalHours",
    "Duration.TotalMinutes",
    "Duration.TotalSeconds",
    "Error.Record",
    "Excel.CurrentWorkbook",
    "Excel.Workbook",
    "File.Contents",
    "Folder.Contents",
    "Folder.Files",
    "Int64.From",
    "Json.Document",
    "Lines.FromBinary",
    "Lines.FromText",
    "Lines.ToText",
    "List.Accumulate",
    "List.AllTrue",
    "List.AnyTrue",
    "List.Average",
    "List.Buffer",
    "List.Combine",
    "List.Contains",
    "List.ContainsAll",
    "List.ContainsAny",
    "List.Count",
    "List.Dates",
    "List.Difference",
    "List.Distinct",
    "List.First",
    "List.FirstN",
    "List.Generate",
    "List.Intersect",
    "List.IsEmpty",
    "List.Last",
    "List.LastN",
    "List.MatchesAll",
    "List.MatchesAny",
    "List.Max",
    "List.Median",
    "List.Min",
    "List.Mode",
    "List.NonNullCount",
    "List.Numbers",
    "List.PositionOf",
    "List.Product",
    "List.Range",
    "List.RemoveFirstN",
    "List.RemoveItems",
    "List.RemoveLastN",
    "List.RemoveNulls",
    "List.Repeat",
    "List.Reverse",
    "List.Select",
    "List.Skip",
    "List.Sort",
    "List.Split",
    "List.StandardDeviation",
    "List.Sum",
    "List.Transform",
    "List.Union",
    "List.Zip",
    "Logical.From",
    "Logical.FromText",
    "Logical.ToText",
    "MySQL.Database",
    "Number.Abs",
    "Number.Exp",
    "Number.From",
    "Number.FromText",
    "Number.IntegerDivide",
    "Number.IsEven",
    "Number.IsNaN",
    "Number.IsOdd",
    "Number.Ln",
    "Number.Log",
    "Number.Log10",
    "Number.Mod",
    "Number.Power",
    "Number.Round",
    "Number.RoundAwayFromZero",
    "Number.RoundDown",
    "Number.RoundTowardZero",
    "Number.RoundUp",
    "Number.Sign",
    "Number.Sqrt",
    "Number.ToText",
    "OData.Feed",
    "Odbc.DataSource",
    "Odbc.Query",
    "Parquet.Document",
    "Percentage.From",
    "PostgreSQL.Database",
    "Python.Execute",
    "Record.AddField",
    "Record.Field",
    "Record.FieldCount",
    "Record.FieldNames",
    "Record.FieldOrDefault",
    "Record.FieldValues",
    "Record.FromList",
    "Record.HasFields",
    "Record.RemoveFields",
    "Record.RenameFields",
    "Record.SelectFields",
    "Record.ToList",
    "Record.ToTable",
    "Replacer.ReplaceText",
    "Replacer.ReplaceValue",
    "Splitter.SplitByNothing",
    "Splitter.SplitTextByAnyDelimiter",
    "Splitter.SplitTextByCharacterTransition",
    "Splitter.SplitTextByDelimiter",
    "Splitter.SplitTextByEachDelimiter",
    "Splitter.SplitTextByLengths",
    "Splitter.SplitTextByPositions",
    "Splitter.SplitTextByWhitespace",
    "Sql.Database",
    "Sql.Databases",
    "Sqlite.Database",
    "Table.AddColumn",
    "Table.AddIndexColumn",
    "Table.AddRankColumn",
    "Table.AggregateTableColumn",
    "Table.AlternateRows",
    "Table.Buffer",
    "Table.Column",
    "Table.ColumnCount",
    "Table.ColumnNames",
    "Table.Combine",
    "Table.CombineColumns",
    "Table.DemoteHeaders",
    "Table.Distinct",
    "Table.DuplicateColumn",
    "Table.ExpandListColumn",
    "Table.ExpandRecordColumn",
    "Table.ExpandTableColumn",
    "Table.FillDown",
    "Table.FillUp",
    "Table.First",
    "Table.FirstN",
    "Table.FromColumns",
    "Table.FromList",
    "Table.FromRecords",
    "Table.FromRows",
    "Table.FromValue",
    "Table.Group",
    "Table.HasColumns",
    "Table.IsEmpty",
    "Table.Join",
    "Table.Last",
    "Table.LastN",
    "Table.Max",
    "Table.Min",
    "Table.NestedJoin",
    "Table.Pivot",
    "Table.PrefixColumns",
    "Table.Profile",
    "Table.PromoteHeaders",
    "Table.Range",
    "Table.RemoveColumns",
    "Table.RemoveFirstN",
    "Table.RemoveLastN",
    "Table.RemoveMatchingRows",
    "Table.RemoveRows",
    "Table.RemoveRowsWithErrors",
    "Table.RenameColumns",
    "Table.ReorderColumns",
    "Table.Repeat",
    "Table.ReplaceErrorValues",
    "Table.ReplaceValue",
    "Table.ReverseRows",
    "Table.RowCount",
    "Table.Schema",
    "Table.SelectColumns",
    "Table.SelectRows",
    "Table.SelectRowsWithErrors",
    "Table.Skip",
    "Table.Sort",
    "Table.SplitColumn",
    "Table.ToColumns",
    "Table.ToList",
    "Table.ToRecords",
    "Table.ToRows",
    "Table.TransformColumnNames",
    "Table.TransformColumnTypes",
    "Table.TransformColumns",
    "Table.TransformRows",
    "Table.Transpose",
    "Table.Unpivot",
    "Table.UnpivotOtherColumns",
    "Text.AfterDelimiter",
    "Text.At",
    "Text.BeforeDelimiter",
    "Text.BetweenDelimiters",
    "Text.Clean",
    "Text.Combine",
    "Text.Contains",
    "Text.End",
    "Text.EndsWith",
    "Text.Format",
    "Text.From",
    "Text.FromBinary",
    "Text.Insert",
    "Text.Length",
    "Text.Lower",
    "Text.Middle",
    "Text.PadEnd",
    "Text.PadStart",
    "Text.PositionOf",
    "Text.PositionOfAny",
    "Text.Proper",
    "Text.Range",
    "Text.Remove",
    "Text.RemoveRange",
    "Text.Repeat",
    "Text.Replace",
    "Text.ReplaceRange",
    "Text.Reverse",
    "Text.Select",
    "Text.Split",
    "Text.SplitAny",
    "Text.Start",
    "Text.StartsWith",
    "Text.ToBinary",
    "Text.ToList",
    "Text.Trim",
    "Text.TrimEnd",
    "Text.TrimStart",
    "Text.Upper",
    "Time.From",
    "Time.FromText",
    "Time.Hour",
    "Time.Minute",
    "Time.Second",
    "Time.ToText",
    "Value.Compare",
    "Value.Equals",
    "Value.Is",
    "Value.ReplaceType",
    "Value.Type",
    "Web.Contents",
    "Web.Page",
    "Xml.Document",
    "Xml.Tables",
];

#[cfg(test)]
mod tests {
    #[test]
    fn names_are_sorted() {
        let mut v = super::NAMES.to_vec();
        v.sort_unstable();
        assert_eq!(v, super::NAMES);
    }
}
