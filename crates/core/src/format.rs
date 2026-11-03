//! The Format pane schema: cards and properties for each element kind. The format pane is
//! generated from this; every property gets an fx button. Charts read the resolved values
//! through [`crate::fx::Props`] using these keys.

use crate::color::Color;
use crate::doc::*;

#[derive(Clone, Debug, PartialEq)]
pub enum PropKind {
    Bool,
    Num { min: f64, max: f64, step: f64 },
    Text,
    /// Multi-line text with `{…}` placeholders.
    Template,
    Color,
    Choice(&'static [&'static str]),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PropDef {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: PropKind,
    pub default: Prop,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub name: &'static str,
    /// "Visual" or "General" tab of the format pane.
    pub tab: &'static str,
    pub props: Vec<PropDef>,
}

pub const FONTS: &[&str] = &["Inter", "Segoe UI", "Arial", "Helvetica", "Georgia", "Verdana", "Courier New", "Trebuchet MS", "Times New Roman", "DIN", "Roboto", "Open Sans"];
pub const POSITIONS: &[&str] = &["Top", "Bottom", "Left", "Right", "Top center", "Bottom center", "Center right"];
pub const ALIGN: &[&str] = &["Left", "Center", "Right"];
pub const DISPLAY_UNITS: &[&str] = &["Auto", "None", "Thousands", "Millions", "Billions", "Trillions"];
pub const LINE_STYLES: &[&str] = &["solid", "dash", "dot"];

fn b(key: &'static str, label: &'static str, v: bool) -> PropDef {
    PropDef { key, label, kind: PropKind::Bool, default: Prop::Bool(v) }
}
fn n(key: &'static str, label: &'static str, v: f64, min: f64, max: f64) -> PropDef {
    PropDef { key, label, kind: PropKind::Num { min, max, step: if max - min > 20.0 { 1.0 } else { 0.1 } }, default: Prop::Num(v) }
}
fn t(key: &'static str, label: &'static str, v: &str) -> PropDef {
    PropDef { key, label, kind: PropKind::Text, default: Prop::Text(v.into()) }
}
fn tpl(key: &'static str, label: &'static str, v: &str) -> PropDef {
    PropDef { key, label, kind: PropKind::Template, default: Prop::Text(v.into()) }
}
fn c(key: &'static str, label: &'static str, v: Option<Color>) -> PropDef {
    PropDef { key, label, kind: PropKind::Color, default: v.map(Prop::Color).unwrap_or(Prop::Text(String::new())) }
}
fn ch(key: &'static str, label: &'static str, opts: &'static [&'static str], v: &str) -> PropDef {
    PropDef { key, label, kind: PropKind::Choice(opts), default: Prop::Choice(v.into()) }
}

/// The schema key of an element: the visual kind's serde name, or "shape", "text", …
pub fn schema_key(el: &Element) -> String {
    match &el.kind {
        ElementKind::Visual(v) => serde_json::to_value(v.kind).ok().and_then(|j| j.as_str().map(String::from)).unwrap_or_default(),
        ElementKind::Shape(_) => "shape".into(),
        ElementKind::Text(_) => "text".into(),
        ElementKind::Image(_) => "image".into(),
        ElementKind::Button(_) => "button".into(),
        ElementKind::Group => "group".into(),
    }
}

fn general() -> Vec<Card> {
    vec![
        Card {
            name: "Title",
            tab: "General",
            props: vec![
                b("title.show", "Title", true),
                tpl("title.text", "Text", ""),
                tpl("subtitle.text", "Subtitle", ""),
                ch("title.font", "Font", FONTS, "Inter"),
                n("title.size", "Text size", 13.0, 6.0, 72.0),
                b("title.bold", "Bold", true),
                c("title.color", "Text color", None),
                c("title.background", "Background color", None),
                ch("title.align", "Horizontal alignment", ALIGN, "Left"),
                b("title.wrap", "Text wrap", true),
                b("divider.show", "Divider", false),
            ],
        },
        Card {
            name: "Effects",
            tab: "General",
            props: vec![
                b("background.show", "Background", true),
                c("background.color", "Background color", None),
                n("background.transparency", "Transparency %", 0.0, 0.0, 100.0),
                b("border.show", "Visual border", false),
                c("border.color", "Border color", None),
                n("border.width", "Border width", 1.0, 0.0, 12.0),
                n("border.radius", "Rounded corners", -1.0, -1.0, 60.0),
                b("shadow.show", "Shadow", false),
                c("shadow.color", "Shadow color", Some(Color::rgba(0, 0, 0, 48))),
                n("shadow.blur", "Shadow blur", 10.0, 0.0, 60.0),
                n("shadow.offset", "Shadow distance", 3.0, 0.0, 30.0),
            ],
        },
        Card {
            name: "Header icons",
            tab: "General",
            props: vec![b("header.show", "Visual header", true), b("header.focus", "Focus mode icon", true), b("header.drill", "Drill icons", true), b("header.openSheet", "Open as sheet (↗)", true)],
        },
        Card { name: "Tooltips", tab: "General", props: vec![b("tooltip.show", "Tooltips", true), ch("tooltip.type", "Type", &["Default", "Report page"], "Default"), c("tooltip.background", "Background", None)] },
        Card { name: "Alt text", tab: "General", props: vec![tpl("altText", "Alt text", "")] },
    ]
}

fn axes(x_label: &'static str, y_label: &'static str) -> Vec<Card> {
    let _ = (x_label, y_label);
    vec![
        Card {
            name: "X-axis",
            tab: "Visual",
            props: vec![
                b("xAxis.show", "X-axis", true),
                ch("xAxis.type", "Type", &["Categorical", "Continuous"], "Categorical"),
                n("xAxis.size", "Font size", 10.0, 6.0, 40.0),
                c("xAxis.color", "Color", None),
                b("xAxis.title", "Title", true),
                tpl("xAxis.titleText", "Title text", ""),
                ch("xAxis.units", "Display units", DISPLAY_UNITS, "Auto"),
                b("xAxis.concatenate", "Concatenate labels", true),
                n("xAxis.maxHeight", "Maximum label area %", 25.0, 0.0, 50.0),
            ],
        },
        Card {
            name: "Y-axis",
            tab: "Visual",
            props: vec![
                b("yAxis.show", "Y-axis", true),
                t("yAxis.min", "Range minimum", ""),
                t("yAxis.max", "Range maximum", ""),
                ch("yAxis.scale", "Scale type", &["Linear", "Log"], "Linear"),
                b("yAxis.invert", "Invert range", false),
                n("yAxis.size", "Font size", 10.0, 6.0, 40.0),
                c("yAxis.color", "Color", None),
                ch("yAxis.units", "Display units", DISPLAY_UNITS, "Auto"),
                t("yAxis.format", "Value format", ""),
                b("yAxis.title", "Title", true),
                tpl("yAxis.titleText", "Title text", ""),
                ch("yAxis.position", "Position", &["Left", "Right"], "Left"),
                b("y2Axis.show", "Secondary y-axis", true),
            ],
        },
        Card {
            name: "Gridlines",
            tab: "Visual",
            props: vec![
                b("grid.horizontal", "Horizontal", true),
                b("grid.vertical", "Vertical", false),
                c("grid.color", "Color", Some(Color::rgb(0xE6, 0xE8, 0xEC))),
                n("grid.width", "Width", 1.0, 0.0, 6.0),
                ch("grid.style", "Line style", LINE_STYLES, "solid"),
            ],
        },
        Card { name: "Zoom slider", tab: "Visual", props: vec![b("zoom.show", "Zoom slider", false)] },
    ]
}

fn legend() -> Card {
    Card {
        name: "Legend",
        tab: "Visual",
        props: vec![
            b("legend.show", "Legend", true),
            ch("legend.position", "Position", POSITIONS, "Top"),
            n("legend.size", "Text size", 10.0, 6.0, 30.0),
            c("legend.color", "Color", None),
            b("legend.title", "Title", true),
            tpl("legend.titleText", "Title text", ""),
            ch("legend.marker", "Marker", &["Circle", "Square", "Line"], "Circle"),
        ],
    }
}

fn data_colors() -> Card {
    Card {
        name: "Data colors",
        tab: "Visual",
        props: vec![
            c("dataColors.default", "Default color", None),
            b("dataColors.showAll", "Show all (per point)", false),
            c("dataColors.highlight", "Highlight color", None),
            n("dataColors.transparency", "Transparency %", 0.0, 0.0, 100.0),
        ],
    }
}

fn labels() -> Card {
    Card {
        name: "Data labels",
        tab: "Visual",
        props: vec![
            b("labels.show", "Data labels", false),
            ch("labels.position", "Position", &["Auto", "Inside end", "Inside center", "Inside base", "Outside end"], "Auto"),
            n("labels.size", "Text size", 9.0, 6.0, 40.0),
            c("labels.color", "Color", None),
            ch("labels.units", "Display units", DISPLAY_UNITS, "Auto"),
            n("labels.decimals", "Value decimal places", -1.0, -1.0, 10.0),
            t("labels.format", "Value format", ""),
            b("labels.background", "Background", false),
            tpl("labels.template", "Custom label ({value}, {category}, {series}, {percent})", ""),
            b("totals.show", "Total labels", false),
        ],
    }
}

fn plot() -> Card {
    Card {
        name: "Plot area",
        tab: "Visual",
        props: vec![
            c("plot.background", "Background", None),
            n("plot.innerPadding", "Space between bars %", 20.0, 0.0, 90.0),
            n("plot.seriesPadding", "Space between series %", 4.0, 0.0, 50.0),
            n("plot.minCategoryWidth", "Minimum category width", 20.0, 0.0, 200.0),
            n("plot.cornerRadius", "Bar corner radius", 2.0, 0.0, 30.0),
        ],
    }
}

fn lines() -> Card {
    Card {
        name: "Lines",
        tab: "Visual",
        props: vec![
            n("lines.width", "Stroke width", 2.5, 0.5, 12.0),
            ch("lines.style", "Line style", LINE_STYLES, "solid"),
            ch("lines.interpolation", "Interpolation", &["Linear", "Smooth", "Step"], "Linear"),
            b("markers.show", "Markers", false),
            n("markers.size", "Marker size", 4.0, 1.0, 20.0),
            ch("markers.shape", "Marker shape", &["Circle", "Square", "Diamond", "Triangle"], "Circle"),
            n("area.transparency", "Area transparency %", 60.0, 0.0, 100.0),
        ],
    }
}

fn text_card(name: &'static str, prefix: &'static str) -> Card {
    // Keys like "values.size": built from static strings for each prefix used below.
    let props = match prefix {
        "callout" => vec![n("callout.size", "Font size", 36.0, 8.0, 160.0), c("callout.color", "Color", None), ch("callout.units", "Display units", DISPLAY_UNITS, "Auto"), t("callout.format", "Value format", ""), ch("callout.font", "Font", FONTS, "Inter"), b("callout.bold", "Bold", true)],
        "category" => vec![b("category.show", "Category label", true), n("category.size", "Font size", 11.0, 6.0, 40.0), c("category.color", "Color", None)],
        "values" => vec![n("values.size", "Font size", 10.0, 6.0, 40.0), c("values.color", "Text color", None), c("values.background", "Background", None), c("values.altBackground", "Alternate background", None), b("values.banded", "Banded rows", true), b("values.wrap", "Text wrap", false)],
        "headers" => vec![n("headers.size", "Font size", 10.0, 6.0, 40.0), c("headers.color", "Text color", None), c("headers.background", "Background", None), b("headers.bold", "Bold", true), ch("headers.align", "Alignment", ALIGN, "Left")],
        _ => vec![],
    };
    Card { name, tab: "Visual", props }
}

pub fn cards(kind: &str) -> Vec<Card> {
    let mut out: Vec<Card> = Vec::new();
    match kind {
        "stackedBar" | "stackedColumn" | "clusteredBar" | "clusteredColumn" | "stackedBar100" | "stackedColumn100" | "ribbon" | "histogram" | "waterfall" => {
            out.extend(axes("X", "Y"));
            out.push(legend());
            out.push(data_colors());
            out.push(labels());
            out.push(plot());
            if kind == "waterfall" {
                out.push(Card {
                    name: "Sentiment colors",
                    tab: "Visual",
                    props: vec![c("waterfall.increase", "Increase", Some(Color::rgb(0x16, 0xA3, 0x4A))), c("waterfall.decrease", "Decrease", Some(Color::rgb(0xDC, 0x26, 0x26))), c("waterfall.total", "Total", Some(Color::rgb(0x2B, 0x6C, 0xE0))), b("waterfall.showTotal", "Show total", true)],
                });
            }
        }
        "line" | "area" | "stackedArea" | "stackedArea100" | "lineStackedColumn" | "lineClusteredColumn" | "dualAxis" | "sparkline" => {
            out.extend(axes("X", "Y"));
            out.push(legend());
            out.push(data_colors());
            out.push(labels());
            out.push(lines());
            out.push(plot());
        }
        "scatter" => {
            out.extend(axes("X", "Y"));
            out.push(legend());
            out.push(data_colors());
            out.push(labels());
            out.push(Card {
                name: "Markers",
                tab: "Visual",
                props: vec![n("markers.size", "Size", 6.0, 1.0, 40.0), ch("markers.shape", "Shape", &["Circle", "Square", "Diamond", "Triangle"], "Circle"), n("markers.transparency", "Transparency %", 15.0, 0.0, 100.0), b("scatter.categoryLabels", "Category labels", false)],
            });
        }
        "pie" | "donut" | "funnel" | "treemap" | "packedBubbles" | "wordCloud" | "sankey" | "radar" => {
            out.push(legend());
            out.push(data_colors());
            out.push(Card {
                name: "Detail labels",
                tab: "Visual",
                props: vec![
                    b("labels.show", "Detail labels", true),
                    ch("labels.content", "Label contents", &["Category", "Data value", "Percent of total", "Category, data value", "Category, percent of total", "All detail labels"], "Category, percent of total"),
                    ch("labels.position", "Position", &["Outside", "Inside", "Prefer outside"], "Prefer outside"),
                    n("labels.size", "Text size", 9.0, 6.0, 40.0),
                    c("labels.color", "Color", None),
                    t("labels.format", "Value format", ""),
                ],
            });
            if kind == "donut" || kind == "pie" {
                out.push(Card { name: "Slices", tab: "Visual", props: vec![n("donut.inner", "Inner radius %", if kind == "donut" { 55.0 } else { 0.0 }, 0.0, 95.0), n("slices.gap", "Spacing", 1.0, 0.0, 10.0), n("slices.rotation", "Rotation", 0.0, 0.0, 360.0)] });
            }
        }
        "map" | "filledMap" => {
            out.push(legend());
            out.push(data_colors());
            out.push(Card { name: "Bubbles", tab: "Visual", props: vec![n("bubbles.size", "Size", 10.0, 1.0, 60.0), c("map.land", "Land color", Some(Color::rgb(0xE9, 0xEE, 0xF3))), c("map.water", "Water color", Some(Color::rgb(0xF7, 0xFA, 0xFC))), b("map.graticule", "Grid lines", true)] });
        }
        "gauge" => {
            out.push(Card { name: "Gauge axis", tab: "Visual", props: vec![t("gauge.min", "Min", ""), t("gauge.max", "Max", ""), t("gauge.target", "Target", "")] });
            out.push(Card { name: "Colors", tab: "Visual", props: vec![c("gauge.fill", "Fill", None), c("gauge.track", "Track", Some(Color::rgb(0xE5, 0xE7, 0xEB))), c("gauge.targetColor", "Target", Some(Color::rgb(0x1F, 0x23, 0x2B)))] });
            out.push(text_card("Callout value", "callout"));
        }
        "card" | "kpi" | "multiRowCard" => {
            out.push(text_card("Callout value", "callout"));
            out.push(text_card("Category label", "category"));
            if kind == "kpi" {
                out.push(Card {
                    name: "KPI",
                    tab: "Visual",
                    props: vec![
                        ch("kpi.direction", "High is", &["Good", "Bad"], "Good"),
                        c("kpi.good", "Good color", Some(Color::rgb(0x16, 0xA3, 0x4A))),
                        c("kpi.neutral", "Neutral color", Some(Color::rgb(0xEA, 0xB3, 0x08))),
                        c("kpi.bad", "Bad color", Some(Color::rgb(0xDC, 0x26, 0x26))),
                        b("kpi.trend", "Trend axis", true),
                        b("kpi.goal", "Show goal", true),
                        b("kpi.distance", "Distance to goal", true),
                    ],
                });
            }
            out.push(Card { name: "Card", tab: "Visual", props: vec![ch("card.align", "Alignment", ALIGN, "Center"), b("card.accentBar", "Accent bar", false), c("card.accentColor", "Accent color", None), tpl("card.reference", "Reference label", "")] });
        }
        "slicer" => {
            out.push(Card {
                name: "Slicer settings",
                tab: "Visual",
                props: vec![
                    ch("slicer.style", "Style", &["Vertical list", "Tile", "Dropdown", "Between", "Less than or equal to", "Greater than or equal to", "Relative date"], "Vertical list"),
                    b("slicer.single", "Single select", false),
                    b("slicer.multiCtrl", "Multi-select with Ctrl", false),
                    b("slicer.selectAll", "Show \"Select all\"", true),
                    b("slicer.search", "Search", true),
                    b("slicer.header", "Slicer header", true),
                ],
            });
            out.push(text_card("Values", "values"));
        }
        "table" | "matrix" | "heatMap" | "highlightTable" => {
            out.push(Card {
                name: "Style presets",
                tab: "Visual",
                props: vec![ch("table.style", "Style", &["Default", "Minimal", "Bold header", "Alternating rows", "Flashy rows", "Condensed", "Contrast", "None"], "Default"), n("table.rowPadding", "Row padding", 4.0, 0.0, 30.0), c("grid.color", "Gridline color", Some(Color::rgb(0xE6, 0xE8, 0xEC)))],
            });
            out.push(text_card("Column headers", "headers"));
            out.push(text_card("Values", "values"));
            out.push(Card {
                name: "Totals",
                tab: "Visual",
                props: vec![b("totals.show", "Totals", true), b("totals.rowSubtotals", "Row subtotals", true), b("totals.columnSubtotals", "Column subtotals", true), c("totals.background", "Background", None), b("totals.bold", "Bold", true)],
            });
            if kind != "table" {
                out.push(Card { name: "Row headers", tab: "Visual", props: vec![b("matrix.stepped", "Stepped layout", true), n("matrix.indent", "Stepped layout indentation", 16.0, 0.0, 60.0), b("matrix.expandIcons", "+/- icons", true)] });
            }
            out.push(Card { name: "Sparklines", tab: "Visual", props: vec![b("sparklines.show", "Sparklines", false), c("sparklines.color", "Color", None)] });
        }
        "keyInfluencers" | "decompositionTree" | "qandA" | "smartNarrative" => {
            out.push(data_colors());
            out.push(Card { name: "Analysis", tab: "Visual", props: vec![ch("ai.type", "Analysis type", &["Categorical", "Continuous"], "Categorical"), b("ai.highValue", "Analyze increase", true), n("ai.maxSplits", "Max levels", 5.0, 1.0, 20.0)] });
        }
        "pythonVisual" | "rVisual" => {
            out.push(Card { name: "Script", tab: "Visual", props: vec![ch("script.renderer", "Interpreter", &["python3", "python", "Rscript"], "python3"), n("script.dpi", "Resolution (dpi)", 110.0, 50.0, 300.0)] });
        }
        "boxPlot" | "bullet" | "gantt" => {
            out.extend(axes("X", "Y"));
            out.push(data_colors());
            out.push(labels());
        }
        "shape" => {
            out.push(Card {
                name: "Shape",
                tab: "Visual",
                props: vec![
                    c("shape.fill", "Fill color", None),
                    n("shape.fillTransparency", "Fill transparency %", 0.0, 0.0, 100.0),
                    c("shape.stroke", "Border color", None),
                    n("shape.strokeWidth", "Border width", -1.0, -1.0, 30.0),
                    n("shape.radius", "Corner radius", -1.0, -1.0, 200.0),
                    n("shape.rotation", "Rotation", 0.0, -360.0, 360.0),
                ],
            });
            out.push(Card { name: "Text", tab: "Visual", props: vec![tpl("shape.text", "Text", ""), n("shape.textSize", "Size", 14.0, 6.0, 120.0), c("shape.textColor", "Color", None), ch("shape.textAlign", "Alignment", ALIGN, "Center")] });
            out.push(Card { name: "Action", tab: "General", props: vec![] });
            return out;
        }
        "text" => {
            out.push(Card { name: "Text box", tab: "Visual", props: vec![tpl("text.override", "Text (template, overrides rich text when set)", ""), ch("text.valign", "Vertical alignment", &["Top", "Middle", "Bottom"], "Top")] });
            out.extend(general().into_iter().filter(|c| c.name == "Effects"));
            return out;
        }
        "image" => {
            out.push(Card { name: "Image", tab: "Visual", props: vec![ch("image.scaling", "Scaling", &["Fit", "Fill", "Normal"], "Fit"), tpl("image.url", "Image URL (template)", "")] });
            out.extend(general().into_iter().filter(|c| c.name == "Effects"));
            return out;
        }
        "button" => {
            out.push(Card {
                name: "Button",
                tab: "Visual",
                props: vec![tpl("button.text", "Text", ""), n("button.textSize", "Text size", 12.0, 6.0, 60.0), c("button.textColor", "Text color", None), c("button.fill", "Fill", None), c("button.hoverFill", "Hover fill", None), n("button.radius", "Corner radius", 6.0, 0.0, 100.0), ch("button.icon", "Icon", &["None", "Back", "Bookmark", "Info", "Help", "Q&A", "Arrow", "Reset"], "None")],
            });
            out.extend(general().into_iter().filter(|c| c.name == "Effects"));
            return out;
        }
        "group" => {
            out.extend(general().into_iter().filter(|c| c.name == "Effects"));
            return out;
        }
        _ => {
            out.push(data_colors());
            out.push(labels());
        }
    }
    out.extend(general());
    out
}

/// Every property key the schema knows for a kind (for validation and the fx dialog).
pub fn prop_def(kind: &str, key: &str) -> Option<PropDef> {
    cards(kind).into_iter().flat_map(|c| c.props).find(|p| p.key == key)
}
