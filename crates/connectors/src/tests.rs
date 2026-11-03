#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("vizcraft-conn-{}-{}", std::process::id(), name));
    let _ = std::fs::create_dir_all(&d);
    d
}

fn eval(m: &str) -> Table {
    match vizcraft_query::evaluate(m, &DesktopHost) {
        Ok(Value::Table(t)) => (*t).clone(),
        other => panic!("not a table: {other:?}"),
    }
}

fn q(p: &std::path::Path) -> String {
    vizcraft_query::value::quote_text(&p.to_string_lossy())
}

#[test]
fn csv_file_with_headers_and_types() {
    let d = tmp("csv");
    let f = d.join("sales.csv");
    std::fs::write(&f, "Region;Units;Note\nEast;10;\"a;b\"\nWest;5;x\n").unwrap();
    let m = format!(
        "let Source = Csv.Document(File.Contents({}), [Delimiter=\";\", Encoding=65001, QuoteStyle=QuoteStyle.Csv]),\n #\"Promoted Headers\" = Table.PromoteHeaders(Source, [PromoteAllScalars=true]),\n #\"Changed Type\" = Table.TransformColumnTypes(#\"Promoted Headers\", {{{{\"Units\", Int64.Type}}}})\nin #\"Changed Type\"",
        q(&f)
    );
    let t = eval(&m);
    assert_eq!(t.columns, vec!["Region", "Units", "Note"]);
    assert_eq!(t.rows.len(), 2);
    assert_eq!(t.cell(0, 1).as_number(), Some(10.0));
    assert_eq!(t.cell(0, 2).display(), "a;b");
}

#[test]
fn csv_latin1_encoding() {
    let d = tmp("latin");
    let f = d.join("l.csv");
    std::fs::write(&f, b"Name\nCaf\xe9\n").unwrap();
    let t = eval(&format!("Table.PromoteHeaders(Csv.Document(File.Contents({}), [Delimiter=\",\", Encoding=1252]))", q(&f)));
    assert_eq!(t.cell(0, 0).display(), "Café");
}

#[test]
fn json_file_to_table() {
    let d = tmp("json");
    let f = d.join("p.json");
    std::fs::write(&f, r#"[{"id": 1, "name": "A"}, {"id": 2, "name": "B", "extra": true}]"#).unwrap();
    let t = eval(&format!("Table.FromRecords(Json.Document(File.Contents({})))", q(&f)));
    assert_eq!(t.rows.len(), 2);
    assert_eq!(t.cell(1, 1).display(), "B");
}

#[test]
fn sqlite_navigation_and_query() {
    let d = tmp("sqlite");
    let f = d.join("shop.db");
    let _ = std::fs::remove_file(&f);
    {
        let c = rusqlite::Connection::open(&f).unwrap();
        c.execute_batch(
            "CREATE TABLE orders (id INTEGER, product TEXT, amount REAL, day DATE);
             INSERT INTO orders VALUES (1, 'Pen', 2.5, '2024-01-02'), (2, 'Ink', 7.0, '2024-01-03'), (3, 'Pen', 2.5, '2024-02-01');
             CREATE TABLE customers (id INTEGER, name TEXT);
             INSERT INTO customers VALUES (1, 'Ann');
             CREATE VIEW pens AS SELECT * FROM orders WHERE product = 'Pen';",
        )
        .unwrap();
    }
    let nav = eval(&format!("Sqlite.Database({})", q(&f)));
    let names: Vec<String> = nav.rows.iter().map(|r| r[0].display()).collect();
    assert_eq!(names, vec!["customers", "orders", "pens"]);
    let t = eval(&format!("Sqlite.Database({}){{[Name=\"orders\"]}}[Data]", q(&f)));
    assert_eq!(t.rows.len(), 3);
    assert!(matches!(t.cell(0, 3), Value::Date(_)), "{:?}", t.cell(0, 3));
    let t = eval(&format!("Sqlite.Database({}, [Query=\"SELECT product, SUM(amount) AS total FROM orders GROUP BY product ORDER BY product\"])", q(&f)));
    assert_eq!(t.columns, vec!["product", "total"]);
    assert_eq!(t.cell(1, 1).as_number(), Some(5.0));
    assert_eq!(db::sqlite_tables(&f.to_string_lossy()).unwrap().len(), 3);
}

#[test]
fn folder_lists_files_recursively() {
    let d = tmp("folder");
    std::fs::create_dir_all(d.join("sub")).unwrap();
    std::fs::write(d.join("a.csv"), "x\n1\n").unwrap();
    std::fs::write(d.join("sub").join("b.csv"), "x\n2\n").unwrap();
    let t = folder_count(&d);
    assert!(t >= 2);
}

fn folder_count(d: &std::path::Path) -> usize {
    let t = eval(&format!("Folder.Files({})", q(d)));
    assert!(t.column_index("Date modified").is_some());
    t.rows.len()
}

#[test]
fn xml_tables() {
    let x = "<orders><order id=\"1\"><item>Pen</item><qty>2</qty></order><order id=\"2\"><item>Ink</item><qty>1</qty></order></orders>";
    let t = xml::tables(x).unwrap();
    assert_eq!(t.rows.len(), 1);
    let Value::Table(inner) = t.cell(0, 1) else { panic!() };
    assert_eq!(inner.columns, vec!["Attribute:id", "item", "qty"]);
    assert_eq!(inner.cell(1, 1).display(), "Ink");
}

#[test]
fn server_connection_strings() {
    let a = Args::parse(&["postgresql://ann:pw@db.example.com:6543/sales".into(), String::new(), "\u{1}opt:CommandTimeout=30".into()]);
    let c = conn_of_for_test(&a);
    assert_eq!((c.host.as_str(), c.port, c.database.as_str(), c.user.as_deref(), c.password.as_deref()), ("db.example.com", Some(6543), "sales", Some("ann"), Some("pw")));
}

fn conn_of_for_test(a: &Args) -> db::Conn {
    // The same parsing the connectors use (through a public helper).
    db::parse_conn(a, "NOPE_USER", "NOPE_PASS")
}
