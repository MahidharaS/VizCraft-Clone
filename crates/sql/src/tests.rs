#![allow(clippy::unwrap_used)]

use vizcraft_core::build::ColType;
use vizcraft_core::scalar::Scalar;

use crate::text;

#[test]
fn split_and_cursor() {
    let s = "select 1; -- one\n\nselect 'a;b' as x;\n/* c; */ select 3";
    let st = text::split_statements(s);
    assert_eq!(st.len(), 3);
    assert_eq!(&s[st[0].clone()], "select 1");
    assert_eq!(&s[st[1].clone()], "select 'a;b' as x");
    assert_eq!(&s[st[2].clone()], "select 3");
    let pos = s.find("'a;b'").unwrap();
    assert_eq!(&s[text::statement_at(s, pos).unwrap()], "select 'a;b' as x");
    assert_eq!(&s[text::statement_at(s, 0).unwrap()], "select 1");
    assert_eq!(&s[text::statement_at(s, s.len()).unwrap()], "select 3");
    assert_eq!(text::rewrite_top("SELECT TOP 5 * FROM t"), "SELECT * FROM t LIMIT 5");
    let toks = text::tokenize("SELECT sum(x) FROM \"T\" WHERE y = 'a' -- c");
    assert!(toks.iter().any(|(_, k)| *k == text::Tok::Function));
    assert!(toks.iter().any(|(_, k)| *k == text::Tok::QuotedIdent));
    assert!(toks.iter().any(|(_, k)| *k == text::Tok::Comment));
    assert_eq!(text::word_before("select Sa", 9), (7, "Sa"));
    assert_eq!(text::qualifier_before("x \"Sales\".Qu", 10).as_deref(), Some("Sales"));
}

#[test]
fn date_serials() {
    // 2024-01-15 is serial 45306.
    assert_eq!(crate::serial_to_date(45306.0).unwrap().to_string(), "2024-01-15");
    assert_eq!(crate::unix_days_to_serial(crate::serial_to_unix_days(45306.0) as i64), 45306.0);
    let dt = 45306.75;
    assert!((crate::unix_micros_to_serial(crate::serial_to_unix_micros(dt)) - dt).abs() < 1e-9);
    assert_eq!(crate::display(&Scalar::Num(45306.5), ColType::DateTime), "2024-01-15 12:00:00");
}

#[cfg(feature = "duckdb")]
mod duck {
    use super::*;
    use std::collections::BTreeMap;
    use vizcraft_core::build::TableData;
    use crate::{Engine, ObjectKind, StatementOutput};

    fn sample() -> vizcraft_core::build::Built {
        let mut p = vizcraft_core::sample::sales();
        vizcraft_core::build::sync_model_tables(&mut p);
        vizcraft_core::build::build(&p, &vizcraft_query::NoHost, &BTreeMap::new())
    }

    #[test]
    fn type_round_trip() {
        let mut e = Engine::open(None).unwrap();
        let t = TableData {
            name: "Typed".into(),
            columns: vec!["d".into(), "ts".into(), "i".into(), "n".into(), "s".into(), "b".into(), "t".into(), "any".into()],
            types: vec![ColType::Date, ColType::DateTime, ColType::Integer, ColType::Number, ColType::Text, ColType::Bool, ColType::Time, ColType::Any],
            rows: vec![
                vec![Scalar::Num(45306.0), Scalar::Num(45306.75), Scalar::Num(42.0), Scalar::Num(2.5), Scalar::Text("héllo".into()), Scalar::Bool(true), Scalar::Num(0.5), Scalar::Num(7.0)],
                vec![Scalar::Null, Scalar::Null, Scalar::Null, Scalar::Null, Scalar::Null, Scalar::Null, Scalar::Null, Scalar::Text("x".into())],
                vec![Scalar::Num(1.0), Scalar::Num(-0.25), Scalar::Num(-3.0), Scalar::Num(1e-3), Scalar::Text("it's".into()), Scalar::Bool(false), Scalar::Num(0.0), Scalar::Null],
            ],
        };
        e.register_table(&t).unwrap();
        let r = e.query("select * from Typed", 100).unwrap();
        assert_eq!(r.types, vec!["DATE", "TIMESTAMP", "BIGINT", "DOUBLE", "VARCHAR", "BOOLEAN", "TIME", "VARCHAR"]);
        assert_eq!(r.col_types[..7], t.types[..7]);
        assert_eq!(r.rows[0][..7], t.rows[0][..7]);
        assert_eq!(r.rows[0][7], Scalar::Text("7".into()));
        assert_eq!(r.rows[1][..7], t.rows[1][..7]);
        assert_eq!(r.rows[2][..7], t.rows[2][..7]);
        // DuckDB sees real dates.
        let r = e.query("select strftime(d, '%Y-%m-%d') as s, year(d) as y, hour(ts) as h from Typed where d is not null order by d", 10).unwrap();
        assert_eq!(r.rows[0][0], Scalar::Text("1899-12-31".into()));
        assert_eq!(r.rows[1][0], Scalar::Text("2024-01-15".into()));
        assert_eq!(r.rows[1][1], Scalar::Num(2024.0));
        assert_eq!(r.rows[1][2], Scalar::Num(18.0));
        // Literals come back typed too.
        let r = e.query("select DATE '2024-01-15' as d, TIMESTAMP '2024-01-15 18:00:00' as ts, 1.5::DECIMAL(5,2) as dec", 1).unwrap();
        assert_eq!(r.col_types, vec![ColType::Date, ColType::DateTime, ColType::Number]);
        assert_eq!(r.rows[0], vec![Scalar::Num(45306.0), Scalar::Num(45306.75), Scalar::Num(1.5)]);
        // As M: dates become M dates.
        let m = r.to_m();
        assert_eq!(m.rows[0][0], vizcraft_query::Value::Date(45306));
    }

    #[test]
    fn sample_tables_and_dax_agree() {
        let b = sample();
        let mut e = Engine::open(None).unwrap();
        let errs = e.sync_model(b.tables.values().map(|t| t.as_ref()));
        assert!(errs.is_empty(), "{errs:?}");
        let cat = e.catalog().unwrap();
        for name in ["Sales", "Product", "Store"] {
            let o = cat.iter().find(|o| o.name == name).unwrap_or_else(|| panic!("{name} missing: {cat:?}"));
            assert_eq!(o.kind, ObjectKind::Model);
            assert!(!o.columns.is_empty());
        }
        let sales = cat.iter().find(|o| o.name == "Sales").unwrap();
        assert!(sales.columns.iter().any(|(c, t)| c == "Date" && t == "DATE"), "{:?}", sales.columns);
        // SUM(Quantity) matches DAX.
        let dax = b.model.evaluate("SUM(Sales[Quantity])").unwrap();
        let r = e.query("select sum(Quantity) from Sales", 10).unwrap();
        assert_eq!(r.rows[0][0].as_f64(), Scalar::from_dm(&dax).as_f64());
        let rows = b.model.evaluate("COUNTROWS(Sales)").unwrap();
        let r = e.query("select count(*) from Sales", 10).unwrap();
        assert_eq!(r.rows[0][0].as_f64(), Scalar::from_dm(&rows).as_f64());
        // A join + group by through the model's relationship keys.
        let r = e.query("select st.Region, sum(s.Quantity) q from Sales s join Store st using (StoreID) group by 1 order by 1", 100);
        if let Ok(r) = r {
            let total: f64 = r.rows.iter().filter_map(|row| row[1].as_f64()).sum();
            assert_eq!(Some(total), Scalar::from_dm(&dax).as_f64());
        }
        // Re-registering unchanged data is skipped; dropping a table removes it.
        let errs = e.sync_model(b.tables.values().filter(|t| t.name != "Targets").map(|t| t.as_ref()));
        assert!(errs.is_empty());
        assert!(!e.is_registered("Targets"));
        assert!(e.query("select * from Targets", 1).is_err());
    }

    #[test]
    fn warehouse_persists() {
        let dir = std::env::temp_dir().join(format!("vizcraft-sql-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("project.duckdb");
        let _ = std::fs::remove_file(&path);
        let b = sample();
        {
            let mut e = Engine::open(Some(&path)).unwrap();
            e.sync_model(b.tables.values().map(|t| t.as_ref()));
            let out = e.run_script("create table top_products as select ProductID, sum(Quantity) as qty from Sales group by 1 order by 2 desc limit 3; select count(*) from top_products", 100);
            assert!(out.iter().all(|o| !matches!(o, StatementOutput::Error { .. })), "{out:?}");
            assert_eq!(e.warehouse_tables(), vec!["top_products".to_string()]);
        }
        // Reopen: the warehouse table is still there; model tables are not (they're per session).
        let e = Engine::open(Some(&path)).unwrap();
        let r = e.query("select count(*) from top_products", 10).unwrap();
        assert_eq!(r.rows[0][0], Scalar::Num(3.0));
        assert!(e.query("select * from Sales", 1).is_err());
        let cat = e.catalog().unwrap();
        assert_eq!(cat.iter().find(|o| o.name == "top_products").map(|o| o.kind), Some(ObjectKind::Warehouse));
        drop(e);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn errors_are_readable() {
        let e = Engine::open(None).unwrap();
        let err = e.query("selec 1", 10).unwrap_err();
        assert!(err.to_lowercase().contains("syntax"), "{err}");
        let err = e.query("select * from nope", 10).unwrap_err();
        assert!(err.contains("nope"), "{err}");
        let out = e.run_script("select 1; select * from nope; select 2", 10);
        assert_eq!(out.len(), 2);
        assert!(matches!(out[1], StatementOutput::Error { .. }));
    }

    #[test]
    fn explain_files_export() {
        let mut e = Engine::open(None).unwrap();
        let plan = e.explain("select 42", false).unwrap();
        assert!(!plan.trim().is_empty());
        let plan = e.explain("select sum(range) from range(1000)", true).unwrap();
        assert!(!plan.trim().is_empty());
        let dir = std::env::temp_dir().join(format!("vizcraft-sql-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let csv = dir.join("people.csv");
        std::fs::write(&csv, "name,age,born\nAda,36,1815-12-10\nAlan,41,1912-06-23\n").unwrap();
        let v = e.attach_file(&csv.to_string_lossy(), None).unwrap();
        assert_eq!(v, "people");
        let r = e.query("select name, born from people order by age", 10).unwrap();
        assert_eq!(r.col_types[1], ColType::Date);
        assert_eq!(r.cell_text(0, 1), "1815-12-10");
        assert!(e.catalog().unwrap().iter().any(|o| o.name == "people" && o.kind == ObjectKind::File));
        let pq = dir.join("people.parquet");
        e.export("select * from people", &pq.to_string_lossy()).unwrap();
        let r = e.query(&format!("select count(*) from read_parquet('{}')", pq.to_string_lossy()), 1).unwrap();
        assert_eq!(r.rows[0][0], Scalar::Num(2.0));
        let _ = std::fs::remove_dir_all(&dir);
        // DML reports rows affected; TOP is rewritten.
        let out = e.run_script("create table t as select * from range(10) r(x); insert into t select * from range(5); select top 3 * from t", 100);
        if let Some(StatementOutput::Rows(r)) = out.get(1) {
            assert_eq!(r.rows_affected, Some(5));
        }
        if let Some(StatementOutput::Rows(r)) = out.get(2) {
            assert_eq!(r.rows.len(), 3);
        }
        // Extensions that can't be installed offline fail with a message, not a crash.
        if let Err(m) = e.attach_database("postgres", "host=localhost dbname=none connect_timeout=1", "pg") {
            assert!(!m.is_empty());
        }
    }

    #[test]
    fn row_cap() {
        let e = Engine::open(None).unwrap();
        let r = e.query("select * from range(1000)", 100).unwrap();
        assert_eq!(r.rows.len(), 100);
        assert!(r.truncated);
    }

    #[test]
    fn worker_runs_and_cancels() {
        let e = Engine::open(None).unwrap();
        let mut w = crate::worker::Worker::new(&e).unwrap();
        w.submit(crate::worker::Job::Script { sql: "select 1 as one; select 2 as two".into(), cap: 10 });
        let d = w.wait(std::time::Duration::from_secs(30)).unwrap();
        assert_eq!(d.outputs.len(), 2);
        w.submit(crate::worker::Job::Script { sql: "select count(*) from range(10000000000) a".into(), cap: 10 });
        std::thread::sleep(std::time::Duration::from_millis(200));
        w.cancel();
        let d = w.wait(std::time::Duration::from_secs(60)).unwrap();
        assert!(matches!(d.outputs.first(), Some(StatementOutput::Error { message, .. }) if message.contains("cancel") || message.to_lowercase().contains("interrupt")), "{:?}", d.outputs);
    }

    /// `VizCraft.Sql` in Power Query, through a host backed by the engine.
    #[test]
    fn m_function() {
        struct H<'a>(&'a Engine);
        impl vizcraft_query::m::eval::Host for H<'_> {
            fn sql(&self, q: &str) -> vizcraft_query::Result<vizcraft_query::Table> {
                self.0.query(q, usize::MAX).map(|r| r.to_m()).map_err(vizcraft_query::Error::Eval)
            }
        }
        let b = sample();
        let mut e = Engine::open(None).unwrap();
        e.sync_model(b.tables.values().map(|t| t.as_ref()));
        let v = vizcraft_query::evaluate("let S = VizCraft.Sql(\"select sum(Quantity) as q from Sales\") in S{0}[q]", &H(&e)).unwrap();
        let dax = b.model.evaluate("SUM(Sales[Quantity])").unwrap();
        assert_eq!(v.as_number(), Scalar::from_dm(&dax).as_f64());
        let err = vizcraft_query::evaluate("VizCraft.Sql(\"select 1\")", &vizcraft_query::NoHost).unwrap_err();
        assert!(err.to_string().contains("SQL"));
    }

    #[test]
    fn python_bridge() {
        if crate::python::interpreter().is_none() {
            return;
        }
        let e = Engine::open(None).unwrap();
        let df = e.query("select range as x, DATE '2024-01-01' + range::INT as d from range(5)", 100).unwrap();
        let out = crate::python::run_blocking("print(len(df))\nout = df.assign(y=df.x * 2)", &df).unwrap();
        if out.stderr.contains("pandas isn't installed") {
            return;
        }
        assert!(out.ok, "{}", out.stderr);
        assert_eq!(out.stdout.trim(), "5");
        let r = out.out.unwrap();
        assert_eq!(r.columns, vec!["x", "d", "y"]);
        assert_eq!(r.col_types, vec![ColType::Integer, ColType::Date, ColType::Integer]);
        assert_eq!(r.rows[4][2], Scalar::Num(8.0));
        assert_eq!(r.cell_text(1, 1), "2024-01-02");
    }
}
