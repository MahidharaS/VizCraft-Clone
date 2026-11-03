//! Renders every visual kind against the Retail sample: no panics, non-empty scenes. The
//! ignored `gallery` test writes SVG contact sheets to /tmp/vizcraft-charts for eyeballing
//! (`cargo test -p vizcraft-core gallery -- --ignored`, then `convert x.svg x.png`).

use std::collections::BTreeMap;

use super::*;
use crate::build::{self, Built};
use crate::scene::Scene;

fn col(t: &str, c: &str) -> FieldUse {
    FieldUse::new(FieldRef::col(t, c))
}
fn ms(n: &str) -> FieldUse {
    FieldUse::new(FieldRef::measure("Sales", n))
}
fn lv(l: &str) -> FieldUse {
    FieldUse::new(FieldRef::Level { table: "Sales".into(), hierarchy: "Date".into(), level: l.into() })
}

/// A representative spec per kind (plus a few variants), with a display name.
pub fn cases() -> Vec<(String, VisualSpec, Vec<(&'static str, Prop)>)> {
    use VisualKind::*;
    let mut out = Vec::new();
    let mut add = |name: &str, kind: VisualKind, wells: Vec<(&str, Vec<FieldUse>)>, props: Vec<(&'static str, Prop)>, f: &dyn Fn(&mut VisualSpec)| {
        let mut v = VisualSpec::new(kind);
        for (r, fs) in wells {
            v.set(r, fs);
        }
        f(&mut v);
        out.push((name.to_string(), v, props));
    };
    let none = |_: &mut VisualSpec| {};
    let month = vec![lv("Year"), lv("Month")];
    let expand = |v: &mut VisualSpec| v.drill.expand_to = 1;
    add("Stacked bar", StackedBar, vec![("axis", vec![col("Store", "Store")]), ("values", vec![ms("Total Sales")]), ("legend", vec![col("Sales", "Channel")])], vec![("totals.show", Prop::Bool(true))], &none);
    add("Stacked column", StackedColumn, vec![("axis", vec![col("Store", "Region")]), ("values", vec![ms("Total Sales")]), ("legend", vec![col("Product", "Category")])], vec![("labels.show", Prop::Bool(true))], &none);
    add("Clustered bar", ClusteredBar, vec![("axis", vec![col("Product", "Product")]), ("values", vec![ms("Total Sales")])], vec![("labels.show", Prop::Bool(true))], &none);
    add("Clustered column + avg", ClusteredColumn, vec![("axis", month.clone()), ("values", vec![ms("Total Sales"), ms("Profit")])], vec![], &|v: &mut VisualSpec| {
        v.analytics.push(AnalyticsLine { kind: "Average".into(), name: "Average sales".into(), ..AnalyticsLine::default() });
    });
    add("100% stacked bar", StackedBar100, vec![("axis", vec![col("Store", "Region")]), ("values", vec![ms("Total Sales")]), ("legend", vec![col("Sales", "Channel")])], vec![("labels.show", Prop::Bool(true))], &none);
    add("100% stacked column", StackedColumn100, vec![("axis", vec![lv("Quarter")]), ("values", vec![ms("Total Sales")]), ("legend", vec![col("Product", "Category")])], vec![], &none);
    add("Line + trend + forecast", Line, vec![("axis", month.clone()), ("values", vec![ms("Total Sales")])], vec![], &|v: &mut VisualSpec| {
        v.drill.expand_to = 1;
        v.analytics.push(AnalyticsLine { kind: "Trend".into(), name: "Trend".into(), ..AnalyticsLine::default() });
        v.analytics.push(AnalyticsLine { kind: "Forecast".into(), name: "Forecast".into(), forecast_length: 6, seasonality: 12, ..AnalyticsLine::default() });
    });
    add("Line legend smooth markers", Line, vec![("axis", vec![lv("Month")]), ("values", vec![ms("Total Sales")]), ("legend", vec![col("Sales", "Channel")])], vec![("lines.interpolation", Prop::Choice("Smooth".into())), ("markers.show", Prop::Bool(true)), ("legend.position", Prop::Choice("Right".into()))], &none);
    add("Area", Area, vec![("axis", vec![lv("Month")]), ("values", vec![ms("Total Sales"), ms("Profit")])], vec![], &none);
    add("Stacked area", StackedArea, vec![("axis", month.clone()), ("values", vec![ms("Total Sales")]), ("legend", vec![col("Product", "Category")])], vec![], &expand);
    add("100% stacked area", StackedArea100, vec![("axis", vec![lv("Month")]), ("values", vec![ms("Total Sales")]), ("legend", vec![col("Sales", "Channel")])], vec![], &none);
    add("Line + stacked column", LineStackedColumn, vec![("axis", vec![lv("Month")]), ("values", vec![ms("Total Sales")]), ("legend", vec![col("Sales", "Channel")]), ("secondary", vec![ms("Margin %")])], vec![], &none);
    add("Line + clustered column", LineClusteredColumn, vec![("axis", vec![col("Store", "Region")]), ("values", vec![ms("Total Sales"), ms("Profit")]), ("secondary", vec![ms("Margin %")])], vec![("labels.show", Prop::Bool(true))], &none);
    add("Ribbon", Ribbon, vec![("axis", vec![lv("Quarter")]), ("values", vec![ms("Total Sales")]), ("legend", vec![col("Product", "Category")])], vec![], &none);
    add("Small multiples", ClusteredColumn, vec![("axis", vec![col("Product", "Category")]), ("values", vec![ms("Total Sales")]), ("small", vec![col("Store", "Region")])], vec![], &none);
    add("Waterfall", Waterfall, vec![("axis", vec![lv("Quarter")]), ("values", vec![ms("Profit")])], vec![("labels.show", Prop::Bool(true))], &none);
    add("Waterfall breakdown", Waterfall, vec![("axis", vec![lv("Year")]), ("breakdown", vec![col("Product", "Category")]), ("values", vec![ms("Total Sales")])], vec![], &none);
    add("Funnel", Funnel, vec![("axis", vec![col("Product", "Category")]), ("values", vec![ms("Orders")])], vec![], &none);
    add("Scatter", Scatter, vec![("details", vec![col("Product", "Product")]), ("x", vec![ms("Units")]), ("y", vec![ms("Margin %")]), ("size", vec![ms("Total Sales")]), ("legend", vec![col("Product", "Category")])], vec![("scatter.categoryLabels", Prop::Bool(true))], &none);
    add("Scatter clusters", Scatter, vec![("details", vec![col("Store", "Store")]), ("x", vec![ms("Orders")]), ("y", vec![ms("Avg Order Value")])], vec![], &|v: &mut VisualSpec| {
        v.analytics.push(AnalyticsLine { kind: "Cluster".into(), clusters: 3, ..AnalyticsLine::default() });
    });
    add("Pie", Pie, vec![("legend", vec![col("Product", "Category")]), ("values", vec![ms("Total Sales")])], vec![], &none);
    add("Donut", Donut, vec![("legend", vec![col("Sales", "Channel")]), ("values", vec![ms("Total Sales")])], vec![], &none);
    add("Treemap", Treemap, vec![("axis", vec![col("Product", "Category")]), ("details", vec![col("Product", "Product")]), ("values", vec![ms("Total Sales")])], vec![], &none);
    add("Map", Map, vec![("location", vec![col("Store", "Store")]), ("lat", vec![FieldUse::agg(FieldRef::col("Store", "Latitude"), Agg::Average)]), ("lon", vec![FieldUse::agg(FieldRef::col("Store", "Longitude"), Agg::Average)]), ("size", vec![ms("Total Sales")])], vec![], &none);
    add("Filled map", FilledMap, vec![("location", vec![col("Store", "Country")]), ("values", vec![ms("Total Sales")])], vec![], &none);
    add("Gauge", Gauge, vec![("values", vec![ms("Total Sales")]), ("target", vec![ms("Sales Target")])], vec![], &none);
    add("Card", Card, vec![("values", vec![ms("Total Sales")])], vec![], &none);
    add("Multi-row card", MultiRowCard, vec![("values", vec![ms("Total Sales"), ms("Profit"), ms("Margin %"), ms("Orders")])], vec![], &none);
    add("Multi-row card by region", MultiRowCard, vec![("values", vec![col("Store", "Region"), ms("Total Sales"), ms("Margin %")])], vec![], &none);
    add("KPI", Kpi, vec![("values", vec![ms("Total Sales")]), ("axis", vec![lv("Month")]), ("target", vec![ms("Sales Target")])], vec![], &none);
    add("Slicer list", Slicer, vec![("field", vec![col("Store", "Region")])], vec![], &|v: &mut VisualSpec| v.selection = vec![Scalar::Text("South".into())]);
    add("Slicer tiles", Slicer, vec![("field", vec![col("Product", "Category")])], vec![("slicer.style", Prop::Choice("Tile".into()))], &none);
    add("Slicer dropdown", Slicer, vec![("field", vec![col("Store", "Store")])], vec![("slicer.style", Prop::Choice("Dropdown".into())), ("slicer.open", Prop::Bool(true))], &none);
    add("Slicer between", Slicer, vec![("field", vec![col("Product", "List Price")])], vec![("slicer.style", Prop::Choice("Between".into()))], &|v: &mut VisualSpec| v.range = Some((40.0, 160.0)));
    add("Table", Table, vec![("columns", vec![col("Store", "Store"), col("Store", "Region"), ms("Total Sales"), ms("Margin %"), ms("Orders")])], vec![
        ("cf.Margin %.background", Prop::Fx(Fx::Gradient { dax: String::new(), min: None, max: None, min_color: crate::color::Color::rgb(0xFE, 0xE2, 0xE2), mid_color: Some(crate::color::Color::rgb(0xFE, 0xF9, 0xC3)), max_color: crate::color::Color::rgb(0xDC, 0xFC, 0xE7) })),
        ("cf.Orders.dataBar", Prop::Fx(Fx::Gradient { dax: String::new(), min: None, max: None, min_color: crate::color::Color::rgb(0x2B, 0x6C, 0xE0), mid_color: None, max_color: crate::color::Color::rgb(0x2B, 0x6C, 0xE0) })),
    ], &none);
    add("Matrix", Matrix, vec![("rows", vec![col("Product", "Category"), col("Product", "Product")]), ("columns", vec![lv("Year")]), ("values", vec![ms("Total Sales"), ms("Margin %")])], vec![], &|v: &mut VisualSpec| v.expanded = vec!["[\"Gear\"]".into()]);
    add("Key influencers", KeyInfluencers, vec![("values", vec![ms("Margin %")]), ("details", vec![col("Store", "Region"), col("Sales", "Channel"), col("Product", "Category")])], vec![], &none);
    add("Decomposition tree", DecompositionTree, vec![("values", vec![ms("Total Sales")]), ("details", vec![col("Store", "Region"), col("Product", "Category"), col("Sales", "Channel")])], vec![], &none);
    add("Q&A", QandA, vec![], vec![], &|v: &mut VisualSpec| v.question = "total sales by region".into());
    add("Smart narrative", SmartNarrative, vec![("values", vec![ms("Total Sales")])], vec![], &none);
    add("Python visual", PythonVisual, vec![("values", vec![ms("Total Sales")])], vec![], &none);
    add("R visual", RVisual, vec![("values", vec![ms("Total Sales")])], vec![], &none);
    add("Histogram", Histogram, vec![("axis", vec![col("Sales", "Unit Price")]), ("values", vec![ms("Orders")])], vec![], &none);
    add("Box plot", BoxPlot, vec![("axis", vec![col("Product", "Category")]), ("details", vec![col("Store", "Store")]), ("values", vec![ms("Total Sales")])], vec![], &none);
    add("Heat map", HeatMap, vec![("rows", vec![col("Store", "Region")]), ("columns", vec![lv("Month")]), ("values", vec![ms("Total Sales")])], vec![], &none);
    add("Highlight table", HighlightTable, vec![("rows", vec![col("Product", "Category")]), ("columns", vec![col("Sales", "Channel")]), ("values", vec![ms("Margin %")])], vec![], &none);
    add("Bullet", Bullet, vec![("axis", vec![col("Store", "Region")]), ("values", vec![ms("Total Sales")]), ("target", vec![ms("Sales Target")])], vec![], &none);
    add("Gantt", Gantt, vec![("axis", vec![col("Product", "Product")]), ("start", vec![FieldUse::agg(FieldRef::col("Sales", "Date"), Agg::Min)]), ("values", vec![ms("Orders")]), ("legend", vec![col("Product", "Category")])], vec![], &none);
    add("Packed bubbles", PackedBubbles, vec![("axis", vec![col("Product", "Product")]), ("values", vec![ms("Total Sales")]), ("legend", vec![col("Product", "Category")])], vec![], &none);
    add("Word cloud", WordCloud, vec![("axis", vec![col("Product", "Product")]), ("values", vec![ms("Units")])], vec![], &none);
    add("Radar", Radar, vec![("axis", vec![col("Store", "Region")]), ("values", vec![ms("Total Sales")]), ("legend", vec![col("Sales", "Channel")])], vec![], &none);
    add("Sankey", Sankey, vec![("source", vec![col("Sales", "Channel")]), ("destination", vec![col("Store", "Region")]), ("values", vec![ms("Total Sales")])], vec![], &none);
    add("Dual axis", DualAxis, vec![("axis", vec![lv("Month")]), ("values", vec![ms("Total Sales")]), ("secondary", vec![ms("Orders")])], vec![], &none);
    add("Sparkline", Sparkline, vec![("axis", vec![lv("Month")]), ("values", vec![ms("Total Sales")])], vec![], &none);
    add("Line anomalies + error bars", ClusteredColumn, vec![("axis", vec![lv("Month")]), ("values", vec![ms("Profit")])], vec![], &|v: &mut VisualSpec| {
        v.analytics.push(AnalyticsLine { kind: "Error bars".into(), ..AnalyticsLine::default() });
        v.analytics.push(AnalyticsLine { kind: "Anomalies".into(), ..AnalyticsLine::default() });
        v.analytics.push(AnalyticsLine { kind: "Median".into(), name: "Median".into(), ..AnalyticsLine::default() });
    });
    out
}

fn setup() -> (Project, Built) {
    let p = crate::sample::sales();
    let b = build::build(&p, &vizcraft_query::NoHost, &BTreeMap::new());
    (p, b)
}

/// Renders a case like the app: chrome + content, as one scene.
pub fn render_case(p: &Project, b: &Built, spec: &VisualSpec, props_in: &[(&'static str, Prop)], w: f32, h: f32, theme: &Theme) -> (Scene, crate::vquery::VisualData) {
    let page = p.report.pages.first().cloned().unwrap_or_default();
    let mut el = Element { id: 999_999, name: spec.kind.label().into(), w, h, kind: ElementKind::Visual(spec.clone()), ..Element::default() };
    for (k, v) in props_in {
        el.props.insert(k.to_string(), v.clone());
    }
    let rt = crate::state::Runtime::default();
    let cx = crate::state::context_for(p, b, &rt, &page, &el);
    let data = crate::vquery::run(p, b, spec, &cx);
    let env = crate::fx::FxEnv::new(p, b, &cx, &page.name);
    let mut props = crate::fx::resolve_props(&env, &el);
    crate::fx::resolve_point_colors(&env, &el, &data, &mut props);
    crate::fx::resolve_cell_colors(&env, &el, &data, &mut props);
    let ch = chrome::chrome(&el, &props, theme);
    let mut scene = ch.scene;
    let o = RenderOpts { theme, props: &props, width: ch.content.w, height: ch.content.h, selected: &[], analytics: &[], script_image: None, search: "" };
    let content = render(spec, &data, &o);
    common::embed(&mut scene, content, ch.content.x, ch.content.y);
    (scene, data)
}

#[test]
fn every_kind_renders() {
    let (p, b) = setup();
    assert!(b.errors.is_empty(), "{:?}", b.errors);
    let theme = Theme::default();
    let cases = cases();
    for k in VisualKind::ALL {
        assert!(cases.iter().any(|(_, v, _)| v.kind == *k), "no test case for {k:?}");
    }
    for (name, spec, props) in &cases {
        for (w, h) in [(480.0, 300.0), (120.0, 60.0), (5.0, 5.0)] {
            let (scene, data) = render_case(&p, &b, spec, props, w, h, &theme);
            if w > 200.0 {
                assert!(data.error.is_none() || spec.kind == VisualKind::PythonVisual, "{name}: {:?}", data.error);
                assert!(scene.items.len() > 2 || scene.message.is_some(), "{name}: empty scene");
            }
        }
    }
}

#[test]
fn empty_and_odd_data_never_panics() {
    let theme = Theme::default();
    let props = crate::fx::Props::default();
    let o = RenderOpts { theme: &theme, props: &props, width: 300.0, height: 200.0, selected: &[], analytics: &[], script_image: None, search: "x" };
    let (_, b) = setup();
    let _ = b;
    let mk = |rows: Vec<Vec<Scalar>>| {
        let mut d = crate::vquery::VisualData::default();
        let c = |label: &str, role: &str, is_value: bool| crate::vquery::DataCol {
            label: label.into(),
            role: role.into(),
            field: FieldRef::col("T", label),
            is_value,
            format: String::new(),
            col_type: crate::build::ColType::Number,
            show_as: ShowAs::Value,
        };
        for (l, r, v) in [("a", "axis", false), ("legend", "legend", false), ("v", "values", true), ("x", "x", true), ("y", "y", true), ("rows", "rows", false), ("columns", "columns", false), ("details", "details", false), ("field", "field", false), ("source", "source", false), ("destination", "destination", false), ("target", "target", true), ("start", "start", true)] {
            d.cols.push(c(l, r, v));
        }
        d.rows = rows;
        d
    };
    let odd = vec![
        vec![],
        vec![vec![Scalar::Null; 13]],
        vec![vec![Scalar::Num(f64::NAN), Scalar::Text("x".into()), Scalar::Num(-5.0), Scalar::Num(f64::INFINITY), Scalar::Num(-1.0), Scalar::Null, Scalar::Null, Scalar::Null, Scalar::Null, Scalar::Null, Scalar::Null, Scalar::Num(0.0), Scalar::Num(0.0)]],
        (0..40).map(|i| vec![Scalar::Num(i as f64), Scalar::Text(format!("L{}", i % 3)), Scalar::Num((i as f64 - 20.0) * 1e9), Scalar::Num(i as f64), Scalar::Num(-(i as f64)), Scalar::Text(format!("R{i}")), Scalar::Text(format!("C{}", i % 4)), Scalar::Text(format!("D{}", i % 5)), Scalar::Num(i as f64), Scalar::Text(format!("S{}", i % 2)), Scalar::Text(format!("T{}", i % 3)), Scalar::Num(0.0), Scalar::Num(i as f64)]).collect(),
    ];
    let selected = vec![vec![(FieldRef::col("T", "a"), Scalar::Num(3.0))]];
    let o_sel = RenderOpts { selected: &selected, ..o };
    for rows in odd {
        let mut d = mk(rows);
        for pass in 0..2 {
            if pass == 1 {
                // Cross-highlight with half values.
                d.highlight = Some(d.rows.iter().map(|r| r.iter().map(|v| v.as_f64().map(|x| Scalar::Num(x / 2.0)).unwrap_or_else(|| v.clone())).collect()).collect());
            }
            for k in VisualKind::ALL {
                let mut spec = VisualSpec::new(*k);
                for kind in ["Forecast", "Trend", "Anomalies", "Error bars", "Cluster", "Average", "Reference band"] {
                    spec.analytics.push(AnalyticsLine { kind: kind.into(), ..AnalyticsLine::default() });
                }
                let _ = render(&spec, &d, &o);
                let _ = render(&spec, &d, &o_sel);
            }
        }
    }
}

#[test]
#[ignore]
fn debug_dates() {
    let (_p, b) = setup();
    let t = b.table("Sales").unwrap();
    eprintln!("cols {:?} types {:?}", t.columns, t.types);
    eprintln!("row0 {:?}", t.rows.first());
}

#[test]
#[ignore]
fn gallery() {
    let (p, b) = setup();
    let dir = std::path::Path::new("/tmp/vizcraft-charts");
    let _ = std::fs::create_dir_all(dir);
    let only = std::env::var("VIZ_ONLY").unwrap_or_default().to_lowercase();
    let theme_name = std::env::var("VIZ_THEME").unwrap_or_else(|_| "VizCraft".into());
    let theme = Theme::by_name(&theme_name).unwrap_or_default();
    let (w, h) = (480.0, 310.0);
    let cases: Vec<_> = cases().into_iter().filter(|(n, _, _)| only.is_empty() || n.to_lowercase().contains(&only)).collect();
    for (si, chunk) in cases.chunks(6).enumerate() {
        let mut body = String::new();
        for (i, (name, spec, props)) in chunk.iter().enumerate() {
            let (mut scene, _) = render_case(&p, &b, spec, props, w, h, &theme);
            scene.width = w;
            let (x, y) = ((i % 3) as f32 * (w + 16.0) + 16.0, (i / 3) as f32 * (h + 36.0) + 28.0);
            body += &format!("<text x=\"{x}\" y=\"{}\" font-size=\"11\" fill=\"#888\">{}</text>", y - 8.0, name.replace('&', "&amp;"));
            body += &format!("<g transform=\"translate({x},{y})\">{}</g>", svg::scene_svg(&scene));
        }
        let doc = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{W}\" height=\"{H}\" font-family=\"DejaVu Sans, Helvetica, Arial, sans-serif\"><rect width=\"100%\" height=\"100%\" fill=\"{bg}\"/>{body}</svg>",
            W = 3.0 * (w + 16.0) + 16.0,
            H = 2.0 * (h + 36.0) + 20.0,
            bg = theme.background.css()
        );
        let _ = std::fs::write(dir.join(format!("sheet{si}.svg")), doc);
    }
}
