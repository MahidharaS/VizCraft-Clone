//! End-to-end tests of the data model on a small star schema.

use vizcraft_datamodel::dates::ymd;
use vizcraft_datamodel::pivot::{PivotSpec, layout};
use vizcraft_datamodel::store::Table;
use vizcraft_datamodel::{CrossFilter, Model, Value};

fn n(x: f64) -> Value {
    Value::Number(x)
}
fn t(s: &str) -> Value {
    Value::text(s)
}
fn table(name: &str, headers: &[&str], rows: Vec<Vec<Value>>) -> Table {
    Table::from_rows(name, &headers.iter().map(|h| h.to_string()).collect::<Vec<_>>(), &rows).unwrap()
}

/// Sales (8 rows) → Products, Customers, Calendar.
fn model() -> Model {
    let d = |y, m, dd| n(ymd(y, m, dd).unwrap());
    let mut m = Model::new();
    m.add_table(table(
        "Sales",
        &["OrderDate", "ShipDate", "ProductId", "CustomerId", "Qty", "Amount"],
        vec![
            vec![d(2025, 1, 10), d(2025, 1, 12), n(1.0), n(100.0), n(2.0), n(20.0)],
            vec![d(2025, 2, 11), d(2025, 3, 2), n(2.0), n(101.0), n(1.0), n(50.0)],
            vec![d(2025, 3, 5), d(2025, 3, 6), n(3.0), n(100.0), n(5.0), n(15.0)],
            vec![d(2026, 1, 20), d(2026, 1, 22), n(1.0), n(102.0), n(3.0), n(30.0)],
            vec![d(2026, 2, 14), d(2026, 2, 20), n(2.0), n(100.0), n(2.0), n(100.0)],
            vec![d(2026, 3, 1), d(2026, 4, 2), n(3.0), n(101.0), n(4.0), n(12.0)],
            vec![d(2026, 3, 9), d(2026, 3, 10), n(2.0), n(102.0), n(1.0), n(50.0)],
            vec![d(2026, 4, 1), d(2026, 4, 3), n(4.0), n(100.0), n(1.0), n(9.0)],
        ],
    ))
    .unwrap();
    m.add_table(table(
        "Products",
        &["ProductId", "Product", "Category", "Price"],
        vec![
            vec![n(1.0), t("Pen"), t("Office"), n(10.0)],
            vec![n(2.0), t("Chair"), t("Furniture"), n(50.0)],
            vec![n(3.0), t("Paper"), t("Office"), n(3.0)],
            vec![n(4.0), t("Lamp"), t("Furniture"), n(9.0)],
            vec![n(5.0), t("Desk"), t("Furniture"), n(200.0)],
        ],
    ))
    .unwrap();
    m.add_table(table(
        "Customers",
        &["CustomerId", "Name", "Region"],
        vec![vec![n(100.0), t("Ann"), t("East")], vec![n(101.0), t("Bob"), t("West")], vec![n(102.0), t("Cy"), t("East")]],
    ))
    .unwrap();
    // A calendar of every day of 2025–2026.
    let start = ymd(2025, 1, 1).unwrap() as i64;
    let end = ymd(2026, 12, 31).unwrap() as i64;
    let mut cal = Vec::new();
    for s in start..=end {
        let dt = vizcraft_datamodel::dates::to_date(s as f64).unwrap();
        use chrono::Datelike;
        cal.push(vec![n(s as f64), n(dt.year() as f64), n(dt.month() as f64)]);
    }
    m.add_table(table("Calendar", &["Date", "Year", "Month"], cal)).unwrap();
    m.add_relationship(("Sales", "ProductId"), ("Products", "ProductId"), CrossFilter::Single, true).unwrap();
    // Given the "wrong" way round: the engine works out which side is the lookup side.
    m.add_relationship(("Customers", "CustomerId"), ("Sales", "CustomerId"), CrossFilter::Single, true).unwrap();
    m.add_relationship(("Sales", "OrderDate"), ("Calendar", "Date"), CrossFilter::Single, true).unwrap();
    m.add_relationship(("Sales", "ShipDate"), ("Calendar", "Date"), CrossFilter::Single, false).unwrap();
    m.add_measure("Sales", "Total Sales", "SUM(Sales[Amount])", None).unwrap();
    m.add_measure("Sales", "Orders", "COUNTROWS(Sales)", None).unwrap();
    m
}

fn eval(m: &Model, e: &str) -> Value {
    m.evaluate(e).unwrap_or_else(|err| panic!("{e}: {err}"))
}
fn evalf(m: &Model, e: &str, f: &[&str]) -> Value {
    m.evaluate_filtered(e, f).unwrap_or_else(|err| panic!("{e}: {err}"))
}

#[test]
fn basic_aggregations() {
    let m = model();
    assert_eq!(eval(&m, "[Total Sales]"), n(286.0));
    assert_eq!(eval(&m, "[Orders]"), n(8.0));
    assert_eq!(eval(&m, "AVERAGE(Sales[Qty])"), n(19.0 / 8.0));
    assert_eq!(eval(&m, "DISTINCTCOUNT(Sales[CustomerId])"), n(3.0));
    assert_eq!(eval(&m, "MAX(Sales[Amount]) - MIN(Sales[Amount])"), n(91.0));
    assert_eq!(eval(&m, "COUNTROWS(Products)"), n(5.0));
    assert_eq!(eval(&m, "DIVIDE([Total Sales], 0, -1)"), n(-1.0));
    assert_eq!(eval(&m, "VAR a = [Total Sales] VAR b = a / 2 RETURN b"), n(143.0));
}

#[test]
fn filters_flow_from_dimensions_to_facts() {
    let m = model();
    assert_eq!(evalf(&m, "[Total Sales]", &["Products[Category] = \"Office\""]), n(77.0));
    assert_eq!(evalf(&m, "[Total Sales]", &["Customers[Region] = \"East\""]), n(224.0));
    assert_eq!(evalf(&m, "[Total Sales]", &["Customers[Region] = \"East\"", "Products[Category] = \"Furniture\""]), n(159.0));
    assert_eq!(evalf(&m, "[Total Sales]", &["Calendar[Year] = 2026"]), n(201.0));
    assert_eq!(evalf(&m, "[Total Sales]", &["Products[Price] >= 50"]), n(200.0));
    assert_eq!(evalf(&m, "[Total Sales]", &["Products[Product] IN {\"Pen\", \"Lamp\"}"]), n(59.0));
    // A filter on the many side does not reach the lookup table with single-direction filtering.
    assert_eq!(evalf(&m, "COUNTROWS(Products)", &["Sales[Qty] > 3"]), n(5.0));
    // Products never sold have no sales.
    assert_eq!(evalf(&m, "[Total Sales]", &["Products[Product] = \"Desk\""]), Value::Blank);
}

#[test]
fn calculate_modifiers() {
    let m = model();
    // Share of all sales, from inside a category filter.
    assert_eq!(evalf(&m, "DIVIDE([Total Sales], CALCULATE([Total Sales], ALL(Products)))", &["Products[Category] = \"Office\""]), n(77.0 / 286.0));
    // FILTER over a lookup table.
    assert_eq!(eval(&m, "CALCULATE([Total Sales], FILTER(Products, Products[Price] < 10))"), n(36.0));
    // KEEPFILTERS intersects instead of replacing.
    assert_eq!(evalf(&m, "CALCULATE([Total Sales], KEEPFILTERS(Products[Category] = \"Office\"))", &["Products[Product] = \"Chair\""]), Value::Blank);
    assert_eq!(evalf(&m, "CALCULATE([Total Sales], Products[Category] = \"Office\")", &["Products[Category] = \"Furniture\""]), n(77.0));
    // ALLEXCEPT keeps the region filter only.
    assert_eq!(
        evalf(
            &m,
            "CALCULATE([Total Sales], ALLEXCEPT(Customers, Customers[Region]))",
            &["Customers[Region] = \"West\"", "Customers[Name] = \"Bob\""]
        ),
        n(62.0)
    );
    assert_eq!(eval(&m, "CALCULATE([Total Sales], REMOVEFILTERS())"), n(286.0));
}

#[test]
fn row_context_and_context_transition() {
    let mut m = model();
    m.add_calculated_column("Sales", "Unit Price", "RELATED(Products[Price])").unwrap();
    m.add_calculated_column("Sales", "List Value", "Sales[Qty] * [Unit Price]").unwrap();
    assert_eq!(eval(&m, "SUM(Sales[List Value])"), n(2.0 * 10.0 + 50.0 + 15.0 + 30.0 + 100.0 + 12.0 + 50.0 + 9.0));
    assert_eq!(eval(&m, "SUMX(Sales, Sales[Qty] * RELATED(Products[Price]))"), n(286.0));
    // A measure inside an iterator is evaluated per row (context transition).
    m.add_calculated_column("Customers", "Customer Sales", "[Total Sales]").unwrap();
    assert_eq!(eval(&m, "SUM(Customers[Customer Sales])"), n(286.0));
    assert_eq!(eval(&m, "MAXX(Customers, [Total Sales])"), n(144.0));
    assert_eq!(eval(&m, "COUNTROWS(FILTER(Customers, [Total Sales] >= 80))"), n(2.0));
    m.add_calculated_column("Products", "Sold", "COUNTROWS(RELATEDTABLE(Sales))").unwrap();
    assert_eq!(eval(&m, "LOOKUPVALUE(Products[Sold], Products[Product], \"Chair\")"), n(3.0));
    m.add_calculated_column("Customers", "Rank", "RANKX(ALL(Customers), [Total Sales])").unwrap();
    assert_eq!(eval(&m, "LOOKUPVALUE(Customers[Rank], Customers[Name], \"Ann\")"), n(1.0));
}

#[test]
fn filter_introspection() {
    let m = model();
    assert_eq!(evalf(&m, "SELECTEDVALUE(Products[Category], \"many\")", &["Products[Product] = \"Pen\""]), t("Office"));
    assert_eq!(eval(&m, "SELECTEDVALUE(Products[Category], \"many\")"), t("many"));
    assert_eq!(evalf(&m, "HASONEVALUE(Customers[Region])", &["Customers[Name] = \"Ann\""]), Value::Bool(true));
    assert_eq!(evalf(&m, "ISFILTERED(Customers[Region])", &["Customers[Name] = \"Ann\""]), Value::Bool(false));
    assert_eq!(evalf(&m, "ISCROSSFILTERED(Sales)", &["Customers[Name] = \"Ann\""]), Value::Bool(true));
    assert_eq!(eval(&m, "IF([Total Sales] > 100, \"big\", \"small\")"), t("big"));
    assert_eq!(eval(&m, "SWITCH(TRUE(), [Orders] < 5, \"few\", [Orders] < 10, \"some\", \"lots\")"), t("some"));
}

#[test]
fn bidirectional_and_inactive_relationships() {
    let mut m = model();
    // Single direction: a customer filter doesn't reach Products.
    assert_eq!(evalf(&m, "COUNTROWS(Products)", &["Customers[Name] = \"Bob\""]), n(5.0));
    let mut both = Model::new();
    for tb in &m.tables {
        both.add_table(tb.clone()).unwrap();
    }
    both.add_relationship(("Sales", "ProductId"), ("Products", "ProductId"), CrossFilter::Both, true).unwrap();
    both.add_relationship(("Sales", "CustomerId"), ("Customers", "CustomerId"), CrossFilter::Single, true).unwrap();
    // Bob bought a chair and paper.
    assert_eq!(both.evaluate_filtered("COUNTROWS(Products)", &["Customers[Name] = \"Bob\""]).unwrap(), n(2.0));
    // Ship date via the inactive relationship: March 2026 orders by order date vs ship date.
    assert_eq!(evalf(&m, "[Total Sales]", &["Calendar[Year] = 2025", "Calendar[Month] = 3"]), n(15.0));
    assert_eq!(
        evalf(&m, "CALCULATE([Total Sales], USERELATIONSHIP(Sales[ShipDate], Calendar[Date]))", &["Calendar[Year] = 2025", "Calendar[Month] = 3"]),
        n(65.0)
    );
    assert!(m.add_relationship(("Sales", "ProductId"), ("Products", "ProductId"), CrossFilter::Single, true).is_err());
    assert!(m.add_relationship(("Sales", "Qty"), ("Sales", "Amount"), CrossFilter::Single, true).is_err());
    assert!(m.add_relationship(("Sales", "Qty"), ("Customers", "Region"), CrossFilter::Single, true).is_err());
}

#[test]
fn time_intelligence() {
    let mut m = model();
    m.add_measure("Sales", "YTD", "TOTALYTD([Total Sales], Calendar[Date])", None).unwrap();
    m.add_measure("Sales", "Last Year", "CALCULATE([Total Sales], SAMEPERIODLASTYEAR(Calendar[Date]))", None).unwrap();
    let mar26 = ["Calendar[Year] = 2026", "Calendar[Month] = 3"];
    assert_eq!(evalf(&m, "[Total Sales]", &mar26), n(62.0));
    assert_eq!(evalf(&m, "[YTD]", &mar26), n(192.0));
    assert_eq!(evalf(&m, "[Last Year]", &mar26), n(15.0));
    assert_eq!(evalf(&m, "CALCULATE([Total Sales], PREVIOUSMONTH(Calendar[Date]))", &mar26), n(100.0));
    assert_eq!(evalf(&m, "CALCULATE([Total Sales], DATEADD(Calendar[Date], -1, MONTH))", &mar26), n(100.0));
    assert_eq!(eval(&m, "YEAR(DATE(2026, 7, 4)) * 100 + MONTH(EOMONTH(DATE(2026, 1, 15), 1))"), n(202602.0));
}

#[test]
fn queries() {
    let m = model();
    let r = m.query("EVALUATE SUMMARIZECOLUMNS(Products[Category], \"Sales\", [Total Sales], \"Orders\", [Orders]) ORDER BY [Sales] DESC").unwrap();
    assert_eq!(r.columns, vec!["Products[Category]", "Sales", "Orders"]);
    assert_eq!(r.rows, vec![vec![t("Furniture"), n(209.0), n(4.0)], vec![t("Office"), n(77.0), n(4.0)]]);
    let r =
        m.query("DEFINE MEASURE Sales[Avg Order] = DIVIDE([Total Sales], [Orders]) EVALUATE TOPN(1, VALUES(Customers[Name]), [Avg Order])").unwrap();
    assert_eq!(r.rows, vec![vec![t("Cy")]]);
    let r = m.query("EVALUATE FILTER(ADDCOLUMNS(VALUES(Customers[Region]), \"S\", [Total Sales]), [S] > 100)").unwrap();
    assert_eq!(r.rows, vec![vec![t("East"), n(224.0)]]);
    // Cross-table grouping drops empty combinations.
    let r = m
        .summarize(
            &[("Customers".into(), "Region".into()), ("Products".into(), "Category".into())],
            &[("S".into(), "[Total Sales]".into())],
            &["Calendar[Year] = 2026".into()],
        )
        .unwrap();
    assert_eq!(r.rows.len(), 3);
    assert_eq!(r.rows[0], vec![t("East"), t("Furniture"), n(159.0)]);
    let r = m.query("EVALUATE SUMMARIZE(Sales, Products[Category], Customers[Region], \"Qty\", SUM(Sales[Qty]))").unwrap();
    assert_eq!(r.rows.len(), 4);
    assert!(m.query("EVALUATE 1 +").is_err());
}

#[test]
fn pivot_layout() {
    let m = model();
    let g = layout(&m, &PivotSpec { rows: vec!["Products[Category]".into()], values: vec!["Total Sales".into()], ..Default::default() }).unwrap();
    assert_eq!(
        g.grid,
        vec![vec![t("Category"), t("Total Sales")], vec![t("Furniture"), n(209.0)], vec![t("Office"), n(77.0)], vec![t("Grand Total"), n(286.0)]]
    );
    let g = layout(
        &m,
        &PivotSpec {
            rows: vec!["Customers[Region]".into()],
            columns: vec!["Calendar[Year]".into()],
            values: vec!["[Total Sales]".into()],
            filters: vec![],
        },
    )
    .unwrap();
    assert_eq!(g.header_rows, 2);
    assert_eq!(g.grid[0], vec![Value::Blank, n(2025.0), n(2026.0), t("Grand Total")]);
    assert_eq!(g.grid[2], vec![t("East"), n(35.0), n(189.0), n(224.0)]);
    assert_eq!(g.grid[3], vec![t("West"), n(50.0), n(12.0), n(62.0)]);
    assert_eq!(g.grid[4], vec![t("Grand Total"), n(85.0), n(201.0), n(286.0)]);
    assert!(layout(&m, &PivotSpec { rows: vec!["Nope[X]".into()], values: vec!["[Total Sales]".into()], ..Default::default() }).is_err());
}

#[test]
fn errors_not_crashes() {
    let mut m = model();
    assert!(m.add_measure("Sales", "Total Sales", "1", None).is_err());
    assert!(m.add_measure("Sales", "Amount", "1", None).is_err());
    assert!(m.add_measure("Nope", "X", "1", None).is_err());
    m.add_measure("Sales", "Loop A", "[Loop B] + 1", None).unwrap();
    m.add_measure("Sales", "Loop B", "[Loop A] + 1", None).unwrap();
    assert!(m.evaluate("[Loop A]").is_err());
    assert!(m.evaluate("Sales[Amount]").is_err());
    assert!(m.evaluate("[Nope]").is_err());
    assert!(m.evaluate("SUM(1)").is_err());
    assert!(m.evaluate("CALCULATE([Total Sales], Sales[Qty] > Products[Price])").is_err());
    assert!(m.evaluate("UNKNOWNFN(1)").is_err());
    assert!(m.add_calculated_column("Sales", "Bad", "1 / \"x\"").is_err());
    assert_eq!(m.evaluate("1/0").unwrap(), n(f64::INFINITY));
    assert!(m.evaluate("COUNTROWS(GENERATESERIES(1, 1e12))").is_err());
}
