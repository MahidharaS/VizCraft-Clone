//! Sample projects built in code (no external files): a retail sales star schema with Power
//! Query queries, a data sheet, relationships, measures and a multi-page report.

use crate::color::Color;
use crate::doc::*;
use crate::scalar::Scalar;
use vizcraft_query::steps::{LoadTo, QueryDef};

/// A small deterministic pseudo-random generator.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % 1_000_000) as f64 / 1_000_000.0
    }
    fn pick(&mut self, n: usize) -> usize {
        ((self.next() * n as f64) as usize).min(n.saturating_sub(1))
    }
}

pub const PRODUCTS: &[(&str, &str, &str, f64, f64)] = &[
    ("P01", "Trail Runner", "Footwear", 120.0, 54.0),
    ("P02", "City Sneaker", "Footwear", 85.0, 36.0),
    ("P03", "Hiking Boot", "Footwear", 160.0, 79.0),
    ("P04", "Rain Jacket", "Apparel", 140.0, 61.0),
    ("P05", "Fleece Hoodie", "Apparel", 70.0, 26.0),
    ("P06", "Merino Tee", "Apparel", 45.0, 15.0),
    ("P07", "Daypack 20L", "Gear", 95.0, 38.0),
    ("P08", "Water Bottle", "Gear", 25.0, 7.0),
    ("P09", "Headlamp", "Gear", 40.0, 14.0),
    ("P10", "Trekking Poles", "Gear", 110.0, 47.0),
    ("P11", "Smart Watch", "Electronics", 299.0, 160.0),
    ("P12", "Action Camera", "Electronics", 349.0, 201.0),
];

pub const STORES: &[(&str, &str, &str, &str, f64, f64)] = &[
    ("S01", "Bengaluru Central", "South", "India", 12.97, 77.59),
    ("S02", "Mumbai Harbour", "West", "India", 19.07, 72.88),
    ("S03", "Delhi Connaught", "North", "India", 28.63, 77.22),
    ("S04", "Chennai Marina", "South", "India", 13.08, 80.27),
    ("S05", "Kolkata Park St", "East", "India", 22.55, 88.35),
    ("S06", "Pune Koregaon", "West", "India", 18.52, 73.86),
    ("S07", "Hyderabad Hitech", "South", "India", 17.44, 78.38),
    ("S08", "Jaipur Pink City", "North", "India", 26.91, 75.79),
];

const CHANNELS: &[&str] = &["Store", "Online", "Marketplace"];

fn m_text(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn products_m() -> String {
    let rows: Vec<String> = PRODUCTS.iter().map(|(id, n, c, p, cost)| format!("{{{}, {}, {}, {p}, {cost}}}", m_text(id), m_text(n), m_text(c))).collect();
    format!(
        "let\n    Source = #table({{\"ProductID\", \"Product\", \"Category\", \"List Price\", \"Unit Cost\"}}, {{{}}}),\n    #\"Changed Type\" = Table.TransformColumnTypes(Source, {{{{\"ProductID\", type text}}, {{\"Product\", type text}}, {{\"Category\", type text}}, {{\"List Price\", type number}}, {{\"Unit Cost\", type number}}}}),\n    #\"Added Margin\" = Table.AddColumn(#\"Changed Type\", \"Unit Margin\", each [List Price] - [Unit Cost], type number)\nin\n    #\"Added Margin\"",
        rows.join(", ")
    )
}

fn stores_m() -> String {
    let rows: Vec<String> = STORES.iter().map(|(id, n, r, c, la, lo)| format!("{{{}, {}, {}, {}, {la}, {lo}}}", m_text(id), m_text(n), m_text(r), m_text(c))).collect();
    format!(
        "let\n    Source = #table({{\"StoreID\", \"Store\", \"Region\", \"Country\", \"Latitude\", \"Longitude\"}}, {{{}}}),\n    #\"Changed Type\" = Table.TransformColumnTypes(Source, {{{{\"Latitude\", type number}}, {{\"Longitude\", type number}}}})\nin\n    #\"Changed Type\"",
        rows.join(", ")
    )
}

fn targets_m() -> String {
    "let\n    Source = #table({\"Region\", \"Target\"}, {{\"South\", 1450000}, {\"West\", 950000}, {\"North\", 900000}, {\"East\", 420000}}),\n    #\"Changed Type\" = Table.TransformColumnTypes(Source, {{\"Target\", type number}})\nin\n    #\"Changed Type\"".to_string()
}

/// The Sales fact table as a data sheet (2 years of orders).
fn sales_sheet() -> DataSheet {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut cells = vec![vec!["OrderID".to_string(), "Date".into(), "ProductID".into(), "StoreID".into(), "Channel".into(), "Quantity".into(), "Unit Price".into(), "Discount".into()]];
    // 2023-01-01 = serial 44927.
    let start = 44927.0;
    let mut id = 10000;
    for day in 0..730 {
        let season = 1.0 + 0.35 * ((day as f64 / 365.0) * std::f64::consts::TAU - 1.2).sin() + if day >= 365 { 0.18 } else { 0.0 };
        let orders = (2.0 + rng.next() * 4.0 * season) as usize;
        for _ in 0..orders {
            id += 1;
            let p = rng.pick(PRODUCTS.len());
            let s = {
                let w = [0.22, 0.18, 0.15, 0.1, 0.06, 0.1, 0.13, 0.06];
                let x = rng.next();
                let mut acc = 0.0;
                let mut k = 0;
                for (i, wi) in w.iter().enumerate() {
                    acc += wi;
                    if x <= acc {
                        k = i;
                        break;
                    }
                }
                k
            };
            let ch = rng.pick(CHANNELS.len());
            let qty = 1 + rng.pick(4);
            let (pid, _, _, price, _) = PRODUCTS.get(p).copied().unwrap_or(("P01", "", "", 100.0, 50.0));
            let sid = STORES.get(s).map(|x| x.0).unwrap_or("S01");
            let disc = if rng.next() < 0.25 { [0.05, 0.1, 0.15, 0.2].get(rng.pick(4)).copied().unwrap_or(0.0) } else { 0.0 };
            let date = crate::numfmt::format_date(start + day as f64, "yyyy-mm-dd");
            cells.push(vec![
                id.to_string(),
                date,
                pid.into(),
                sid.into(),
                CHANNELS.get(ch).copied().unwrap_or("Store").into(),
                qty.to_string(),
                format!("{price}"),
                format!("{disc}"),
            ]);
        }
    }
    DataSheet {
        name: "Sales".into(),
        cells,
        col_widths: vec![],
        col_formats: vec![String::new(), "yyyy-mm-dd".into(), String::new(), String::new(), String::new(), "#,0".into(), "$#,0.00".into(), "0%".into()],
        col_types: vec!["Whole Number".into(), "Date".into(), "Text".into(), "Text".into(), "Text".into(), "Whole Number".into(), "Decimal Number".into(), "Decimal Number".into()],
        load: true,
        from_query: None,
        description: "Order lines (data sheet: edit it like a spreadsheet).".into(),
    }
}

fn fu(t: &str, c: &str) -> FieldUse {
    FieldUse::new(FieldRef::col(t, c))
}
fn ms(name: &str) -> FieldUse {
    FieldUse::new(FieldRef::measure("Sales", name))
}
fn level(t: &str, h: &str, l: &str) -> FieldUse {
    FieldUse::new(FieldRef::Level { table: t.into(), hierarchy: h.into(), level: l.into() })
}

fn visual(p: &mut Project, kind: VisualKind, rect: (f32, f32, f32, f32), wells: Vec<(&str, Vec<FieldUse>)>) -> Element {
    let mut v = VisualSpec::new(kind);
    for (r, f) in wells {
        v.set(r, f);
    }
    Element { id: p.next_id(), name: kind.label().into(), x: rect.0, y: rect.1, w: rect.2, h: rect.3, kind: ElementKind::Visual(v), ..Element::default() }
}

/// The retail sales sample.
pub fn sales() -> Project {
    let mut p = Project { name: "Retail Sales (sample)".into(), ..Project::default() };
    p.model.auto_date_time = true;
    p.queries.queries.push(QueryDef { name: "Product".into(), m: products_m(), load: LoadTo::Model, loaded_table: None, group: Some("Dimensions".into()), description: "Product catalogue".into() });
    p.queries.queries.push(QueryDef { name: "Store".into(), m: stores_m(), load: LoadTo::Model, loaded_table: None, group: Some("Dimensions".into()), description: "Stores and regions".into() });
    p.queries.queries.push(QueryDef { name: "Targets".into(), m: targets_m(), load: LoadTo::Model, loaded_table: None, group: Some("Planning".into()), description: "Annual sales targets per region".into() });
    p.sheets.push(sales_sheet());
    crate::build::sync_model_tables(&mut p);
    let rel = |p: &mut Project, ft: &str, fc: &str, tt: &str, tc: &str| {
        let id = p.next_id();
        p.model.relationships.push(Relationship { id, from_table: ft.into(), from_column: fc.into(), to_table: tt.into(), to_column: tc.into(), active: true, ..Relationship::default() });
    };
    rel(&mut p, "Sales", "ProductID", "Product", "ProductID");
    rel(&mut p, "Sales", "StoreID", "Store", "StoreID");
    let mut layout = DiagramLayout { name: "All tables".into(), zoom: 1.0, ..DiagramLayout::default() };
    layout.positions.insert("Sales".into(), (360.0, 120.0));
    layout.positions.insert("Product".into(), (60.0, 60.0));
    layout.positions.insert("Store".into(), (680.0, 60.0));
    layout.positions.insert("Targets".into(), (680.0, 360.0));
    p.model.layouts.push(layout);
    for (name, expr, fmt) in [
        ("Total Sales", "SUMX(Sales, Sales[Quantity] * Sales[Unit Price] * (1 - Sales[Discount]))", "$#,0"),
        ("Total Cost", "SUMX(Sales, Sales[Quantity] * RELATED(Product[Unit Cost]))", "$#,0"),
        ("Profit", "[Total Sales] - [Total Cost]", "$#,0"),
        ("Margin %", "DIVIDE([Profit], [Total Sales])", "0.0%"),
        ("Orders", "DISTINCTCOUNT(Sales[OrderID])", "#,0"),
        ("Units", "SUM(Sales[Quantity])", "#,0"),
        ("Avg Order Value", "DIVIDE([Total Sales], [Orders])", "$#,0.00"),
        ("Sales Target", "SUM(Targets[Target])", "$#,0"),
    ] {
        p.model.measures.push(Measure { table: "Sales".into(), name: name.into(), expression: expr.into(), format: fmt.into(), ..Measure::default() });
    }
    // Columns: formats.
    if let Some(t) = p.model.tables.iter_mut().find(|t| t.name == "Sales") {
        t.columns.push(ColumnProps { name: "Unit Price".into(), format: "$#,0.00".into(), ..ColumnProps::default() });
        t.columns.push(ColumnProps { name: "Discount".into(), format: "0%".into(), summarize_by: "Average".into(), ..ColumnProps::default() });
        t.hierarchies.push(Hierarchy { name: "Date".into(), levels: vec!["Year".into(), "Quarter".into(), "Month".into(), "Day".into()] });
    }
    p.report.theme = crate::color::Theme::default();
    p.report.theme.background = Color::rgb(0xF3, 0xF5, 0xF8);

    // Page 1: Overview
    let pid = p.next_id();
    let mut page = Page::new(pid, "Overview", PageKind::Report);
    page.background = Fill::Solid(Color::rgb(0xF3, 0xF5, 0xF8));
    let mut header = Element {
        id: p.next_id(),
        name: "Header".into(),
        x: 0.0,
        y: 0.0,
        w: 1280.0,
        h: 64.0,
        kind: ElementKind::Shape(ShapeSpec {
            shape: ShapeKind::Rectangle,
            fill: Fill::Linear { angle: 90.0, stops: vec![(0.0, Color::rgb(0x1E, 0x3A, 0x8A)), (1.0, Color::rgb(0x2B, 0x6C, 0xE0))] },
            ..ShapeSpec::default()
        }),
        ..Element::default()
    };
    header.style.padding = 0.0;
    page.elements.push(header);
    let title = Element {
        id: p.next_id(),
        name: "Title".into(),
        x: 24.0,
        y: 10.0,
        w: 760.0,
        h: 44.0,
        kind: ElementKind::Text(TextSpec {
            text: RichText {
                runs: vec![
                    TextRun { text: "Retail performance · ".into(), size: 22.0, bold: true, color: Some(Prop::Color(Color::WHITE)), ..TextRun::default() },
                    TextRun { text: "{[Total Sales]:$#,0.0,,\"M\"} sales, ".into(), size: 22.0, color: Some(Prop::Color(Color::rgb(0xBF, 0xDB, 0xFE))), ..TextRun::default() },
                    TextRun {
                        text: "{[Margin %]:0.0%} margin".into(),
                        size: 22.0,
                        bold: true,
                        color: Some(Prop::Fx(Fx::Rules {
                            dax: "[Margin %]".into(),
                            rules: vec![FxRule { op: ">=".into(), value: 0.5, value2: 0.0, text: String::new(), percent: false, output: Prop::Color(Color::rgb(0x86, 0xEF, 0xAC)) }],
                            default: Box::new(Prop::Color(Color::rgb(0xFC, 0xA5, 0xA5))),
                        })),
                        ..TextRun::default()
                    },
                ],
                align: "left".into(),
                valign: "middle".into(),
                line_height: 1.2,
            },
            smart_narrative: false,
        }),
        ..Element::default()
    };
    page.elements.push(title);
    let cards = [("Total Sales", 24.0), ("Profit", 278.0), ("Margin %", 532.0), ("Orders", 786.0)];
    for (m, x) in cards {
        let e = visual(&mut p, VisualKind::Card, (x, 84.0, 238.0, 110.0), vec![("values", vec![ms(m)])]);
        page.elements.push(e);
    }
    let mut kpi = visual(&mut p, VisualKind::Kpi, (1040.0, 84.0, 216.0, 110.0), vec![("values", vec![ms("Total Sales")]), ("axis", vec![level("Sales", "Date", "Month")]), ("target", vec![ms("Sales Target")])]);
    kpi.name = "Sales vs target".into();
    page.elements.push(kpi);
    let mut col = visual(&mut p, VisualKind::ClusteredColumn, (24.0, 212.0, 616.0, 300.0), vec![("axis", vec![level("Sales", "Date", "Year"), level("Sales", "Date", "Quarter"), level("Sales", "Date", "Month")]), ("values", vec![ms("Total Sales"), ms("Profit")])]);
    col.props.insert("title.text".into(), Prop::Fx(Fx::Template { text: "Sales and profit by {Selected:Region} region(s)".into() }));
    if let Some(v) = col.visual_mut() {
        v.analytics.push(AnalyticsLine { kind: "Average".into(), name: "Average sales".into(), ..AnalyticsLine::default() });
    }
    page.elements.push(col);
    let mut bar = visual(&mut p, VisualKind::ClusteredBar, (656.0, 212.0, 600.0, 300.0), vec![("axis", vec![fu("Product", "Product")]), ("values", vec![ms("Total Sales")])]);
    bar.props.insert(
        "dataColors.fx".into(),
        Prop::Fx(Fx::Gradient { dax: "[Margin %]".into(), min: None, max: None, min_color: Color::rgb(0xFD, 0xBA, 0x74), mid_color: None, max_color: Color::rgb(0x1E, 0x55, 0xBA) }),
    );
    bar.props.insert("subtitle.text".into(), Prop::Text("Colour = margin % (fx gradient)".into()));
    page.elements.push(bar);
    let mut slicer = visual(&mut p, VisualKind::Slicer, (24.0, 528.0, 200.0, 172.0), vec![("field", vec![fu("Store", "Region")])]);
    slicer.sync_group = Some("Region".into());
    page.elements.push(slicer);
    let donut = visual(&mut p, VisualKind::Donut, (240.0, 528.0, 330.0, 172.0), vec![("legend", vec![fu("Sales", "Channel")]), ("values", vec![ms("Total Sales")])]);
    page.elements.push(donut);
    let mut table = visual(&mut p, VisualKind::Table, (586.0, 528.0, 670.0, 172.0), vec![("columns", vec![fu("Store", "Store"), fu("Store", "Region"), ms("Total Sales"), ms("Margin %"), ms("Orders")])]);
    table.props.insert(
        "cf.Margin %.background".into(),
        Prop::Fx(Fx::Gradient { dax: String::new(), min: None, max: None, min_color: Color::rgb(0xFE, 0xE2, 0xE2), mid_color: Some(Color::rgb(0xFE, 0xF9, 0xC3)), max_color: Color::rgb(0xDC, 0xFC, 0xE7) }),
    );
    page.elements.push(table);
    p.report.pages.push(page);

    // Page 2: Trends
    let pid2 = p.next_id();
    let mut page2 = Page::new(pid2, "Trends & forecast", PageKind::Report);
    let mut line = visual(&mut p, VisualKind::Line, (24.0, 24.0, 820.0, 340.0), vec![("axis", vec![level("Sales", "Date", "Year"), level("Sales", "Date", "Month")]), ("values", vec![ms("Total Sales")])]);
    if let Some(v) = line.visual_mut() {
        v.drill.expand_to = 1;
        v.analytics.push(AnalyticsLine { kind: "Trend".into(), name: "Trend".into(), ..AnalyticsLine::default() });
        v.analytics.push(AnalyticsLine { kind: "Forecast".into(), name: "Forecast".into(), forecast_length: 6, seasonality: 12, ..AnalyticsLine::default() });
    }
    page2.elements.push(line);
    let sc = visual(&mut p, VisualKind::Scatter, (860.0, 24.0, 396.0, 340.0), vec![("details", vec![fu("Product", "Product")]), ("x", vec![ms("Units")]), ("y", vec![ms("Margin %")]), ("size", vec![ms("Total Sales")]), ("legend", vec![fu("Product", "Category")])]);
    page2.elements.push(sc);
    let mx = visual(&mut p, VisualKind::Matrix, (24.0, 380.0, 820.0, 320.0), vec![("rows", vec![fu("Product", "Category"), fu("Product", "Product")]), ("columns", vec![level("Sales", "Date", "Year")]), ("values", vec![ms("Total Sales"), ms("Margin %")])]);
    page2.elements.push(mx);
    let tm = visual(&mut p, VisualKind::Treemap, (860.0, 380.0, 396.0, 320.0), vec![("axis", vec![fu("Product", "Category")]), ("details", vec![fu("Product", "Product")]), ("values", vec![ms("Total Sales")])]);
    page2.elements.push(tm);
    let mut s2 = visual(&mut p, VisualKind::Slicer, (0.0, 0.0, 10.0, 10.0), vec![("field", vec![fu("Store", "Region")])]);
    s2.sync_group = Some("Region".into());
    s2.hidden = true;
    page2.elements.push(s2);
    p.report.pages.push(page2);

    // Page 3: a Tableau-style worksheet.
    let pid3 = p.next_id();
    let mut ws = Page::new(pid3, "Sheet: Sales by store", PageKind::Worksheet);
    let wv = visual(&mut p, VisualKind::StackedBar, (0.0, 0.0, 1280.0, 720.0), vec![("axis", vec![fu("Store", "Store")]), ("values", vec![ms("Total Sales")]), ("legend", vec![fu("Sales", "Channel")])]);
    ws.worksheet_visual = Some(wv.id);
    ws.elements.push(wv);
    p.report.pages.push(ws);

    // Story.
    let sid = p.next_id();
    let p1 = p.next_id();
    let p2 = p.next_id();
    p.report.stories.push(Story {
        id: sid,
        name: "Story 1".into(),
        title: "How the year went".into(),
        points: vec![
            StoryPoint { id: p1, caption: "Sales grew year over year".into(), page: pid, state: ViewState::default(), notes: vec![] },
            StoryPoint { id: p2, caption: "Seasonality and the forecast".into(), page: pid2, state: ViewState::default(), notes: vec![] },
        ],
        width: 1280.0,
        height: 720.0,
        navigator_style: "Caption boxes".into(),
        background: Fill::Solid(Color::WHITE),
    });
    // A what-if parameter.
    p.model.parameters.push(Parameter { name: "Discount scenario".into(), kind: ParameterKind::WhatIf { min: 0.0, max: 0.3, step: 0.05, default: 0.0 }, current: Scalar::Num(0.0), description: "What if we gave an extra discount?".into() });
    p.model.measures.push(Measure { table: "Sales".into(), name: "Scenario Sales".into(), expression: "[Total Sales] * (1 - [Discount scenario Value])".into(), format: "$#,0".into(), ..Measure::default() });
    crate::build::sync_model_tables(&mut p);
    p.publish.interactive = true;
    p.publish.include_mobile = true;
    // A Fabric-style flow: store × product segments with k-means.
    let seg = crate::flow::example_segments(&mut p);
    p.flows.push(seg);
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build;
    use std::collections::BTreeMap;

    #[test]
    fn sample_builds_and_queries() {
        let p = sales();
        let b = build::build(&p, &vizcraft_query::NoHost, &BTreeMap::new());
        assert!(b.errors.is_empty(), "{:?}", b.errors);
        let page = p.report.pages.first().unwrap();
        let rt = crate::state::Runtime::default();
        for el in &page.elements {
            if let Some(v) = el.visual() {
                let cx = crate::state::context_for(&p, &b, &rt, page, el);
                let d = crate::vquery::run(&p, &b, v, &cx);
                assert!(d.error.is_none(), "{} {:?}", el.name, d.error);
                assert!(!d.rows.is_empty(), "{} has no rows", el.name);
            }
        }
        let yr = b.model.evaluate("CALCULATE([Total Sales], 'Sales'[Date.Year] = 2024)").unwrap();
        assert!(yr.as_number().unwrap() > 1000.0, "date levels: {yr:?}");
        let v = b.model.evaluate("[Total Sales]").unwrap();
        assert!(v.as_number().unwrap() > 100000.0);
    }
}
