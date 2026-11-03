//! End-to-end M tests: typical Power Query queries evaluated by the engine.

use vizcraft_query::{NoHost, Value, evaluate};

fn eval(src: &str) -> Value {
    evaluate(src, &NoHost).unwrap_or_else(|e| panic!("{src}\n{e}"))
}

fn table(src: &str) -> vizcraft_query::Table {
    match eval(src) {
        Value::Table(t) => (*t).clone(),
        other => panic!("not a table: {other:?}"),
    }
}

fn col(t: &vizcraft_query::Table, name: &str) -> Vec<String> {
    let c = t.column_index(name).unwrap_or_else(|| panic!("no column {name} in {:?}", t.columns));
    t.rows.iter().map(|r| r[c].display()).collect()
}

const SALES: &str = r#"#table({"Region", "Product", "Date", "Units", "Price"}, {
    {"East", "Pen", "2024-01-15", "10", "1.5"},
    {"West", "Pen", "2024-02-10", "5", "1.5"},
    {"East", "Book", "2024-02-20", "2", "12"},
    {"North", "Book", "2024-03-05", null, "12"},
    {"West", "Lamp", "2024-03-30", "1", "30"}
})"#;

#[test]
fn language_basics() {
    assert_eq!(eval("1 + 2 * 3"), Value::Number(7.0));
    assert_eq!(eval("\"a\" & \"b\""), Value::text("ab"));
    assert_eq!(eval("let x = 2, y = x * 5 in y - 1"), Value::Number(9.0));
    assert_eq!(eval("if 3 > 2 then \"yes\" else \"no\""), Value::text("yes"));
    assert_eq!(eval("let f = (x) => x * x in f(4)"), Value::Number(16.0));
    assert_eq!(eval("List.Sum(List.Transform({1..4}, each _ * 10))"), Value::Number(100.0));
    assert_eq!(eval("[a = 1, b = 2][b]"), Value::Number(2.0));
    assert_eq!(eval("{10, 20, 30}{1}"), Value::Number(20.0));
    assert_eq!(eval("try 1 / \"x\" otherwise -1"), Value::Number(-1.0));
    assert_eq!(eval("null + 1"), Value::Null);
    assert_eq!(eval("null ?? 5"), Value::Number(5.0));
    assert_eq!(eval("#date(2024, 1, 31) + #duration(1, 0, 0, 0)"), eval("#date(2024, 2, 1)"));
    assert_eq!(eval("Date.ToText(#date(2024, 3, 9), \"dd MMM yyyy\")"), Value::text("09 Mar 2024"));
    assert_eq!(eval("Text.Proper(\"hello wORLD\")"), Value::text("Hello World"));
    assert_eq!(eval("Number.Round(2.5)"), Value::Number(2.0));
    assert_eq!(eval("Number.Round(2.345, 2, RoundingMode.AwayFromZero)"), Value::Number(2.35));
    assert!(evaluate("let a = b, b = a in a", &NoHost).is_err());
    assert!(evaluate("Unknown.Function(1)", &NoHost).is_err());
}

#[test]
fn typical_query() {
    let q = format!(
        r#"let
    Source = {SALES},
    #"Changed Type" = Table.TransformColumnTypes(Source, {{{{"Date", type date}}, {{"Units", Int64.Type}}, {{"Price", type number}}}}),
    #"Replaced Value" = Table.ReplaceValue(#"Changed Type", null, 0, Replacer.ReplaceValue, {{"Units"}}),
    #"Added Custom" = Table.AddColumn(#"Replaced Value", "Revenue", each [Units] * [Price], type number),
    #"Filtered Rows" = Table.SelectRows(#"Added Custom", each [Revenue] > 0),
    #"Sorted Rows" = Table.Sort(#"Filtered Rows", {{{{"Revenue", Order.Descending}}}})
in
    #"Sorted Rows""#
    );
    let t = table(&q);
    assert_eq!(col(&t, "Revenue"), vec!["30", "24", "15", "7.5"]);
    assert_eq!(col(&t, "Date")[0], "2024-03-30");
}

#[test]
fn reshaping() {
    let typed = format!("Table.TransformColumnTypes({SALES}, {{{{\"Units\", Int64.Type}}, {{\"Price\", type number}}}})");
    let g = table(&format!(
        "Table.Group({typed}, {{\"Region\"}}, {{{{\"Units\", each List.Sum([Units]), type number}}, {{\"Rows\", each Table.RowCount(_), Int64.Type}}}})"
    ));
    assert_eq!(col(&g, "Region"), vec!["East", "West", "North"]);
    assert_eq!(col(&g, "Units"), vec!["12", "6", "null"]);
    assert_eq!(col(&g, "Rows"), vec!["2", "2", "1"]);
    let p = table(&format!(
        "Table.Pivot(Table.SelectColumns({typed}, {{\"Region\", \"Product\", \"Units\"}}), {{\"Pen\", \"Book\", \"Lamp\"}}, \"Product\", \"Units\", List.Sum)"
    ));
    assert_eq!(p.columns, vec!["Region", "Pen", "Book", "Lamp"]);
    assert_eq!(col(&p, "Pen"), vec!["10", "5", "null"]);
    let u = table(&format!(
        "Table.UnpivotOtherColumns(Table.SelectColumns({SALES}, {{\"Region\", \"Units\", \"Price\"}}), {{\"Region\"}}, \"Attribute\", \"Value\")"
    ));
    assert_eq!(u.rows.len(), 9);
    assert_eq!(u.columns, vec!["Region", "Attribute", "Value"]);
    let s = table(&format!("Table.SplitColumn({SALES}, \"Date\", Splitter.SplitTextByDelimiter(\"-\"), {{\"Y\", \"M\", \"D\"}})"));
    assert_eq!(s.columns, vec!["Region", "Product", "Y", "M", "D", "Units", "Price"]);
    assert_eq!(col(&s, "M")[1], "02");
    let tr = table("Table.Transpose(#table({\"A\", \"B\"}, {{1, 2}, {3, 4}}))");
    assert_eq!(col(&tr, "Column2"), vec!["3", "4"]);
    let f = table("Table.FillDown(#table({\"A\"}, {{1}, {null}, {2}, {null}}), {\"A\"})");
    assert_eq!(col(&f, "A"), vec!["1", "1", "2", "2"]);
    let promoted = table("Table.PromoteHeaders(#table({\"Column1\", \"Column2\"}, {{\"Name\", \"Name\"}, {\"x\", \"y\"}}))");
    assert_eq!(promoted.columns, vec!["Name", "Name_1"]);
}

#[test]
fn merges_and_appends() {
    let left = "#table({\"Id\", \"Name\"}, {{1, \"a\"}, {2, \"b\"}, {3, \"c\"}})";
    let right = "#table({\"Id\", \"Score\"}, {{1, 10}, {1, 11}, {3, 30}, {4, 40}})";
    let j = table(&format!(
        "Table.ExpandTableColumn(Table.NestedJoin({left}, {{\"Id\"}}, {right}, {{\"Id\"}}, \"R\", JoinKind.LeftOuter), \"R\", {{\"Score\"}}, {{\"R.Score\"}})"
    ));
    assert_eq!(col(&j, "Name"), vec!["a", "a", "b", "c"]);
    assert_eq!(col(&j, "R.Score"), vec!["10", "11", "null", "30"]);
    let inner = table(&format!("Table.NestedJoin({left}, {{\"Id\"}}, {right}, {{\"Id\"}}, \"R\", JoinKind.Inner)"));
    assert_eq!(inner.rows.len(), 2);
    let anti = table(&format!("Table.NestedJoin({left}, {{\"Id\"}}, {right}, {{\"Id\"}}, \"R\", JoinKind.LeftAnti)"));
    assert_eq!(col(&anti, "Name"), vec!["b"]);
    let full = table(&format!("Table.NestedJoin({left}, {{\"Id\"}}, {right}, {{\"Id\"}}, \"R\", JoinKind.FullOuter)"));
    assert_eq!(full.rows.len(), 4);
    let app = table(&format!("Table.Combine({{{left}, {right}}})"));
    assert_eq!(app.columns, vec!["Id", "Name", "Score"]);
    assert_eq!(app.rows.len(), 7);
}

#[test]
fn cell_errors_stay_in_cells() {
    let t = table("Table.AddColumn(#table({\"A\"}, {{\"1\"}, {\"x\"}}), \"N\", each Number.From([A]) * 2)");
    assert_eq!(col(&t, "N"), vec!["2", "Error"]);
    let clean = table("Table.RemoveRowsWithErrors(Table.TransformColumnTypes(#table({\"A\"}, {{\"1\"}, {\"x\"}}), {{\"A\", type number}}))");
    assert_eq!(clean.rows.len(), 1);
    let replaced =
        table("Table.ReplaceErrorValues(Table.TransformColumnTypes(#table({\"A\"}, {{\"1\"}, {\"x\"}}), {{\"A\", type number}}), {{\"A\", 0}})");
    assert_eq!(col(&replaced, "A"), vec!["1", "0"]);
}

#[test]
fn csv_and_json_sources() {
    let t = table("Table.PromoteHeaders(Csv.Document(\"Name,Qty#(lf)\"\"Smith, J\"\",3#(lf)Lee,4\"))");
    assert_eq!(t.columns, vec!["Name", "Qty"]);
    assert_eq!(col(&t, "Name"), vec!["Smith, J", "Lee"]);
    let j = table("Table.FromRecords(Json.Document(\"[{\"\"a\"\": 1, \"\"b\"\": \"\"x\"\"}, {\"\"a\"\": 2}]\"))");
    assert_eq!(col(&j, "b"), vec!["x", "null"]);
}
