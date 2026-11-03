//! Coverage of the wider DAX function library: statistics, percentiles, set operations, table
//! constructors, more time intelligence, text and math functions.

use vizcraft_datamodel::dates::ymd;
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

fn model() -> Model {
    let d = |y, m, dd| n(ymd(y, m, dd).unwrap());
    let mut m = Model::new();
    m.add_table(table(
        "Sales",
        &["OrderDate", "ProductId", "Qty", "Amount"],
        vec![
            vec![d(2025, 1, 10), n(1.0), n(2.0), n(20.0)],
            vec![d(2025, 2, 11), n(2.0), n(1.0), n(50.0)],
            vec![d(2025, 3, 5), n(3.0), n(5.0), n(15.0)],
            vec![d(2026, 1, 20), n(1.0), n(3.0), n(30.0)],
            vec![d(2026, 2, 14), n(2.0), n(2.0), n(100.0)],
            vec![d(2026, 3, 1), n(3.0), n(4.0), n(12.0)],
        ],
    ))
    .unwrap();
    m.add_table(table(
        "Products",
        &["ProductId", "Product", "Category"],
        vec![vec![n(1.0), t("Pen"), t("Office")], vec![n(2.0), t("Chair"), t("Furniture")], vec![n(3.0), t("Paper"), t("Office")], vec![n(4.0), t("Desk"), t("Furniture")]],
    ))
    .unwrap();
    let start = ymd(2025, 1, 1).unwrap() as i64;
    let end = ymd(2026, 12, 31).unwrap() as i64;
    let cal: Vec<Vec<Value>> = (start..=end).map(|s| vec![n(s as f64)]).collect();
    m.add_table(table("Calendar", &["Date"], cal)).unwrap();
    m.add_relationship(("Sales", "ProductId"), ("Products", "ProductId"), CrossFilter::Single, true).unwrap();
    m.add_relationship(("Sales", "OrderDate"), ("Calendar", "Date"), CrossFilter::Single, true).unwrap();
    m.add_measure("Sales", "Total", "SUM(Sales[Amount])", None).unwrap();
    m
}

fn eval(m: &Model, e: &str) -> Value {
    m.evaluate(e).unwrap_or_else(|err| panic!("{e}: {err}"))
}
fn num(m: &Model, e: &str) -> f64 {
    eval(m, e).as_number().unwrap_or_else(|| panic!("{e} is not a number"))
}
fn evalf(m: &Model, e: &str, f: &[&str]) -> Value {
    m.evaluate_filtered(e, f).unwrap_or_else(|err| panic!("{e}: {err}"))
}
fn rows(m: &Model, q: &str) -> usize {
    m.query(q).unwrap_or_else(|err| panic!("{q}: {err}")).rows.len()
}

#[test]
fn statistics() {
    let m = model();
    // Amounts: 20, 50, 15, 30, 100, 12 → mean 37.833…
    let v: Vec<f64> = vec![20.0, 50.0, 15.0, 30.0, 100.0, 12.0];
    let mean = v.iter().sum::<f64>() / 6.0;
    let ss: f64 = v.iter().map(|x| (x - mean).powi(2)).sum();
    assert!((num(&m, "VAR.P(Sales[Amount])") - ss / 6.0).abs() < 1e-9);
    assert!((num(&m, "VAR.S(Sales[Amount])") - ss / 5.0).abs() < 1e-9);
    assert!((num(&m, "STDEV.P(Sales[Amount])") - (ss / 6.0).sqrt()).abs() < 1e-9);
    assert!((num(&m, "STDEV.S(Sales[Amount])") - (ss / 5.0).sqrt()).abs() < 1e-9);
    assert!((num(&m, "STDEVX.P(Sales, Sales[Amount])") - (ss / 6.0).sqrt()).abs() < 1e-9);
    assert!((num(&m, "VARX.P(Sales, Sales[Amount])") - ss / 6.0).abs() < 1e-9);
    // Sorted: 12 15 20 30 50 100.
    assert_eq!(num(&m, "PERCENTILE.INC(Sales[Amount], 0.5)"), 25.0);
    assert_eq!(num(&m, "PERCENTILE.INC(Sales[Amount], 1)"), 100.0);
    assert_eq!(num(&m, "PERCENTILEX.INC(Sales, Sales[Amount], 0)"), 12.0);
    assert_eq!(num(&m, "PERCENTILE.EXC(Sales[Amount], 0.5)"), 25.0);
    assert!(m.evaluate("PERCENTILE.EXC(Sales[Amount], 0.01)").is_err());
    assert_eq!(num(&m, "MAXA(Sales[Amount])"), 100.0);
    assert_eq!(num(&m, "MINA(Sales[Qty])"), 1.0);
    assert_eq!(num(&m, "RANK.EQ(50, Sales[Amount])"), 2.0);
    assert_eq!(num(&m, "RANK.EQ(50, Sales[Amount], ASC)"), 5.0);
}

#[test]
fn first_last_contains_scope() {
    let m = model();
    assert_eq!(eval(&m, "FIRSTNONBLANK(Calendar[Date], [Total])"), n(ymd(2025, 1, 10).unwrap()));
    assert_eq!(eval(&m, "LASTNONBLANK(Calendar[Date], [Total])"), n(ymd(2026, 3, 1).unwrap()));
    assert_eq!(eval(&m, "LASTNONBLANKVALUE(Calendar[Date], [Total])"), n(12.0));
    assert_eq!(eval(&m, "CONTAINS(Products, Products[Product], \"Desk\")"), Value::Bool(true));
    assert_eq!(eval(&m, "CONTAINS(Products, Products[Product], \"Lamp\")"), Value::Bool(false));
    assert_eq!(evalf(&m, "ISINSCOPE(Products[Category])", &["Products[Category] = \"Office\""]), Value::Bool(true));
    assert_eq!(eval(&m, "ISINSCOPE(Products[Category])"), Value::Bool(false));
    assert_eq!(eval(&m, "CALCULATE([Total], TREATAS({\"Office\"}, Products[Category]))"), n(77.0));
    assert_eq!(eval(&m, "CALCULATE([Total], CROSSFILTER(Sales[ProductId], Products[ProductId], BOTH))"), n(227.0));
    assert_eq!(eval(&m, "ISONORAFTER(2, 1)"), Value::Bool(true));
    assert_eq!(eval(&m, "ISONORAFTER(1, 2, DESC)"), Value::Bool(true));
    assert_eq!(eval(&m, "ISONORAFTER(\"a\", \"a\", ASC, 1, 2, ASC)"), Value::Bool(false));
}

#[test]
fn dates_and_time_intelligence() {
    let m = model();
    assert_eq!(eval(&m, "DATEDIFF(DATE(2025, 1, 31), DATE(2025, 3, 1), MONTH)"), n(2.0));
    assert_eq!(eval(&m, "DATEDIFF(DATE(2025, 1, 1), DATE(2025, 1, 31), DAY)"), n(30.0));
    assert_eq!(eval(&m, "DATEDIFF(DATE(2024, 12, 31), DATE(2026, 1, 1), YEAR)"), n(2.0));
    assert_eq!(eval(&m, "DATEDIFF(DATE(2025, 1, 1), DATE(2025, 7, 1), QUARTER)"), n(2.0));
    assert_eq!(eval(&m, "EDATE(DATE(2025, 1, 31), 1)"), n(ymd(2025, 2, 28).unwrap()));
    assert!(num(&m, "YEAR(TODAY())") >= 2024.0);
    assert!(num(&m, "NOW()") >= num(&m, "TODAY()"));
    assert!(num(&m, "UTCNOW()") > 40000.0);
    assert_eq!(rows(&m, "EVALUATE CALENDAR(DATE(2025, 1, 1), DATE(2025, 1, 31))"), 31);
    assert_eq!(rows(&m, "CALENDARAUTO()"), 730);
    let mar26 = ["Calendar[Date] >= DATE(2026, 3, 1) && Calendar[Date] <= DATE(2026, 3, 31)"];
    let fm = |e: &str| evalf(&m, &format!("CALCULATE({e})"), &mar26);
    assert_eq!(fm("[Total], DATESINPERIOD(Calendar[Date], DATE(2026, 3, 31), -3, MONTH)"), n(142.0));
    assert_eq!(fm("[Total], PARALLELPERIOD(Calendar[Date], -12, MONTH)"), n(15.0));
    assert_eq!(fm("[Total], PARALLELPERIOD(Calendar[Date], -1, YEAR)"), n(85.0));
    assert_eq!(fm("[Total], NEXTMONTH(Calendar[Date])"), Value::Blank);
    assert_eq!(evalf(&m, "CALCULATE([Total], NEXTMONTH(Calendar[Date]))", &["Calendar[Date] = DATE(2026, 1, 5)"]), n(100.0));
    assert_eq!(evalf(&m, "CALCULATE([Total], NEXTYEAR(Calendar[Date]))", &["Calendar[Date] = DATE(2025, 6, 1)"]), n(142.0));
    assert_eq!(evalf(&m, "STARTOFMONTH(Calendar[Date])", &mar26), n(ymd(2026, 3, 1).unwrap()));
    assert_eq!(evalf(&m, "ENDOFQUARTER(Calendar[Date])", &mar26), n(ymd(2026, 3, 31).unwrap()));
    assert_eq!(evalf(&m, "STARTOFYEAR(Calendar[Date])", &mar26), n(ymd(2026, 1, 1).unwrap()));
    assert_eq!(evalf(&m, "ENDOFYEAR(Calendar[Date])", &mar26), n(ymd(2026, 12, 31).unwrap()));
    assert_eq!(evalf(&m, "LASTDATE(Calendar[Date])", &mar26), n(ymd(2026, 3, 31).unwrap()));
    assert_eq!(eval(&m, "WEEKDAY(DATE(2026, 1, 5), 2)"), n(1.0));
    assert_eq!(eval(&m, "HOUR(0.75) * 100 + MINUTE(TIME(10, 45, 0))"), n(1845.0));
}

#[test]
fn tables() {
    let m = model();
    assert_eq!(rows(&m, "EVALUATE UNION(VALUES(Products[Category]), {\"Garden\"})"), 3);
    assert_eq!(rows(&m, "EVALUATE INTERSECT(VALUES(Products[Product]), {\"Pen\", \"Lamp\"})"), 1);
    assert_eq!(rows(&m, "EVALUATE EXCEPT(VALUES(Products[Product]), {\"Pen\", \"Lamp\"})"), 3);
    assert_eq!(rows(&m, "EVALUATE CROSSJOIN(VALUES(Products[Category]), {1, 2, 3})"), 6);
    let r = m.query("EVALUATE NATURALINNERJOIN(Sales, Products)").unwrap();
    assert_eq!(r.rows.len(), 6);
    assert_eq!(r.columns.len(), 6);
    assert_eq!(rows(&m, "EVALUATE GENERATE(VALUES(Products[Category]), {1, 2})"), 4);
    let r = m.query("EVALUATE ROW(\"A\", 1, \"B\", \"x\")").unwrap();
    assert_eq!(r.columns, vec!["A".to_string(), "B".to_string()]);
    assert_eq!(r.rows, vec![vec![n(1.0), t("x")]]);
    let r = m.query("EVALUATE DATATABLE(\"Name\", STRING, \"Score\", INTEGER, {{\"a\", 1.4}, {\"b\", 2}})").unwrap();
    assert_eq!(r.rows, vec![vec![t("a"), n(1.0)], vec![t("b"), n(2.0)]]);
    assert_eq!(rows(&m, "EVALUATE SAMPLE(2, Sales, Sales[Amount])"), 2);
    assert_eq!(num(&m, "COUNTROWS(ALLSELECTED(Products))"), 4.0);
}

#[test]
fn earlier_in_calculated_columns() {
    let mut m = model();
    m.add_calculated_column("Sales", "Running", "SUMX(FILTER(Sales, Sales[OrderDate] <= EARLIER(Sales[OrderDate])), Sales[Amount])").unwrap();
    assert_eq!(num(&m, "MAX(Sales[Running])"), 227.0);
    m.add_calculated_column("Sales", "Year", "VAR d = Sales[OrderDate] RETURN YEAR(d)").unwrap();
    assert_eq!(num(&m, "MIN(Sales[Year])"), 2025.0);
}

#[test]
fn scalar_functions() {
    let m = model();
    assert_eq!(eval(&m, "EVEN(3.2) + ODD(2) + EVEN(-1)"), n(4.0 + 3.0 - 2.0));
    assert_eq!(eval(&m, "CEILING(4.1, 2) + FLOOR(4.9, 2) + MROUND(7, 3)"), n(6.0 + 4.0 + 6.0));
    assert_eq!(eval(&m, "GCD(12, 18) * 100 + LCM(4, 6)"), n(612.0));
    assert_eq!(eval(&m, "QUOTIENT(7, 2)"), n(3.0));
    let r = num(&m, "RAND()");
    assert!((0.0..1.0).contains(&r));
    let rb = num(&m, "RANDBETWEEN(5, 7)");
    assert!((5.0..=7.0).contains(&rb));
    assert_eq!(eval(&m, "ISERROR(1 / 0)"), Value::Bool(true));
    assert_eq!(eval(&m, "ISERROR(VALUE(\"x\"))"), Value::Bool(true));
    assert_eq!(eval(&m, "ISERROR(1)"), Value::Bool(false));
    assert_eq!(eval(&m, "ISLOGICAL(TRUE())"), Value::Bool(true));
    assert_eq!(eval(&m, "BLANK()"), Value::Blank);
    assert_eq!(eval(&m, "SEARCH(\"N\", \"printer\")"), n(4.0));
    assert_eq!(eval(&m, "SEARCH(\"z\", \"printer\", 1, -1)"), n(-1.0));
    assert_eq!(eval(&m, "FIND(\"t\", \"tat\", 2)"), n(3.0));
    assert!(m.evaluate("FIND(\"T\", \"tat\")").is_err());
    assert_eq!(eval(&m, "UNICHAR(65) & UNICODE(\"B\")"), t("A66"));
    assert_eq!(eval(&m, "FIXED(1234.567, 1)"), t("1,234.6"));
    assert_eq!(eval(&m, "FIXED(1234.567, 0, TRUE)"), t("1235"));
    assert_eq!(eval(&m, "CONCATENATE(\"a\", \"b\")"), t("ab"));
    assert_eq!(eval(&m, "COMBINEVALUES(\"|\", \"a\", 1, \"c\")"), t("a|1|c"));
    assert_eq!(eval(&m, "FORMAT(DATE(2026, 3, 9), \"dd mmm yyyy\")"), t("09 Mar 2026"));
    assert_eq!(eval(&m, "FORMAT(0.1234, \"0.0%\")"), t("12.3%"));
    assert_eq!(eval(&m, "FORMAT(1234567, \"#,##0,K\")"), t("1,235K"));
    assert_eq!(eval(&m, "FORMAT(42, \"Currency\")"), t("$42.00"));
    assert_eq!(eval(&m, "REPLACE(\"abcdef\", 2, 3, \"X\")"), t("aXef"));
    assert!(m.evaluate("SQRT(-1)").unwrap_err().to_string().contains("negative"));
}
