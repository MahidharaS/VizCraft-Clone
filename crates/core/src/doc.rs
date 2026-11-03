//! The VizCraft project document, saved as JSON (`.vizc`).
//!
//! One project holds the whole chain: data acquisition (Power Query M queries and Excel-like
//! data sheets), the semantic model (relationships, measures, calculated columns and tables,
//! hierarchies, parameters, roles), the report (Power BI pages, Tableau worksheets, dashboards
//! and stories, with visuals, shapes, text, images and buttons), bookmarks, flows (Fabric/KNIME
//! style pipelines) and publish targets.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use vizcraft_query::steps::QueryDoc;

use crate::color::{Color, Theme};
use crate::scalar::Scalar;

pub const FILE_EXTENSION: &str = "vizc";
pub const FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Project {
    pub format_version: u32,
    pub name: String,
    /// Power Query: queries (M) and query parameters.
    pub queries: QueryDoc,
    /// Excel-like data sheets (entered or edited data). Each loads to the model as a table.
    pub sheets: Vec<DataSheet>,
    pub model: ModelDef,
    pub report: Report,
    pub flows: Vec<Flow>,
    pub publish: PublishSettings,
    /// Next id handed out by [`Project::next_id`].
    pub id_seed: u64,
}

impl Default for Project {
    fn default() -> Self {
        Project {
            format_version: FORMAT_VERSION,
            name: "Untitled".into(),
            queries: QueryDoc::default(),
            sheets: vec![],
            model: ModelDef::default(),
            report: Report::default(),
            flows: vec![],
            publish: PublishSettings::default(),
            id_seed: 1000,
        }
    }
}

impl Project {
    pub fn new() -> Project {
        let mut p = Project::default();
        let id = p.next_id();
        p.report.pages.push(Page::new(id, "Page 1", PageKind::Report));
        p
    }

    pub fn next_id(&mut self) -> u64 {
        self.id_seed = self.id_seed.saturating_add(1);
        self.id_seed
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_json(s: &str) -> crate::Result<Project> {
        serde_json::from_str(s).map_err(|e| crate::Error::Msg(format!("This isn't a VizCraft project: {e}")))
    }

    pub fn page(&self, id: u64) -> Option<&Page> {
        self.report.pages.iter().find(|p| p.id == id)
    }
    pub fn page_mut(&mut self, id: u64) -> Option<&mut Page> {
        self.report.pages.iter_mut().find(|p| p.id == id)
    }
    /// The element with `id` on any page, and its page id.
    pub fn element(&self, id: u64) -> Option<(u64, &Element)> {
        self.report.pages.iter().find_map(|p| p.elements.iter().find(|e| e.id == id).map(|e| (p.id, e)))
    }
    pub fn element_mut(&mut self, id: u64) -> Option<&mut Element> {
        self.report.pages.iter_mut().find_map(|p| p.elements.iter_mut().find(|e| e.id == id))
    }
    pub fn sheet(&self, name: &str) -> Option<&DataSheet> {
        self.sheets.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }
    pub fn sheet_mut(&mut self, name: &str) -> Option<&mut DataSheet> {
        self.sheets.iter_mut().find(|s| s.name.eq_ignore_ascii_case(name))
    }
    pub fn parameter(&self, name: &str) -> Option<&Parameter> {
        self.model.parameters.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }
    /// A table name (query, sheet or model table) not yet used.
    pub fn free_table_name(&self, base: &str) -> String {
        let taken = |n: &str| {
            self.model.tables.iter().any(|t| t.name.eq_ignore_ascii_case(n))
                || self.queries.queries.iter().any(|q| q.name.eq_ignore_ascii_case(n))
                || self.sheets.iter().any(|s| s.name.eq_ignore_ascii_case(n))
        };
        if !taken(base) {
            return base.to_string();
        }
        (2..10_000).map(|i| format!("{base} ({i})")).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
    }
}

// ---------------------------------------------------------------------------------------------
// Data sheets (Excel-like data entry)
// ---------------------------------------------------------------------------------------------

/// An editable grid of data, like a worksheet. Cells hold raw input: text, numbers or
/// formulas (`=A2*B2`, `=SUM(C2:C10)`), evaluated by [`crate::sheet`]. Row 0 holds headers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DataSheet {
    pub name: String,
    /// `cells[row][col]` raw input. Row 0 are the column headers.
    pub cells: Vec<Vec<String>>,
    /// Column widths in points (UI).
    pub col_widths: Vec<f32>,
    /// Column types forced by the user ("Text", "Number", "Date", "Boolean"); empty = infer.
    pub col_types: Vec<String>,
    /// Load the sheet to the model as a table.
    pub load: bool,
    /// When the sheet started as a copy of a query's output, its name.
    pub from_query: Option<String>,
    pub description: String,
}

// ---------------------------------------------------------------------------------------------
// Semantic model
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelDef {
    pub tables: Vec<ModelTable>,
    pub relationships: Vec<Relationship>,
    pub measures: Vec<Measure>,
    pub calculated_columns: Vec<CalcColumn>,
    pub parameters: Vec<Parameter>,
    pub roles: Vec<Role>,
    /// Model-view layouts (diagram tabs): name → table positions.
    pub layouts: Vec<DiagramLayout>,
    pub culture: String,
    /// Build Year/Quarter/Month/Day columns for date columns (auto date/time).
    pub auto_date_time: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelTable {
    pub name: String,
    pub source: TableSource,
    pub hidden: bool,
    pub description: String,
    pub columns: Vec<ColumnProps>,
    pub hierarchies: Vec<Hierarchy>,
    /// "Import" or "DirectQuery" (DirectQuery is re-evaluated on each refresh).
    pub storage_mode: String,
    pub is_date_table: bool,
    /// The date column of a table marked as a date table.
    pub date_column: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "name")]
pub enum TableSource {
    /// The output of a Power Query query.
    Query(String),
    /// A data sheet.
    Sheet(String),
    /// A DAX table expression (calculated table).
    Calculated(String),
    /// Output of a flow node (flow name / node id as "flow::node").
    Flow(String),
    /// Generated by a parameter.
    Parameter(String),
    #[default]
    None,
}

/// Column-level metadata the user sets in Table view / Model view.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ColumnProps {
    pub name: String,
    /// "Text", "Whole Number", "Decimal Number", "Fixed Decimal", "Date", "Date/Time", "True/False".
    pub data_type: String,
    /// Number/date format string (`#,0.00`, `$#,0`, `0.0%`, `yyyy-mm-dd`, …).
    pub format: String,
    /// Default aggregation: "Sum", "Average", "Min", "Max", "Count", "Distinct count", "None".
    pub summarize_by: String,
    pub sort_by: Option<String>,
    /// "Uncategorized", "Address", "City", "Country", "Latitude", "Longitude", "Web URL", "Image URL", …
    pub data_category: String,
    pub hidden: bool,
    pub display_folder: String,
    pub description: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Hierarchy {
    pub name: String,
    pub levels: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Cardinality {
    #[default]
    ManyToOne,
    OneToOne,
    OneToMany,
    ManyToMany,
}

impl Cardinality {
    pub const ALL: [Cardinality; 4] = [Cardinality::ManyToOne, Cardinality::OneToOne, Cardinality::OneToMany, Cardinality::ManyToMany];
    pub fn label(self) -> &'static str {
        match self {
            Cardinality::ManyToOne => "Many to one (*:1)",
            Cardinality::OneToOne => "One to one (1:1)",
            Cardinality::OneToMany => "One to many (1:*)",
            Cardinality::ManyToMany => "Many to many (*:*)",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CrossFilterDir {
    #[default]
    Single,
    Both,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Relationship {
    pub id: u64,
    pub from_table: String,
    pub from_column: String,
    pub to_table: String,
    pub to_column: String,
    pub cardinality: Cardinality,
    pub cross_filter: CrossFilterDir,
    pub active: bool,
    /// Apply security filter in both directions.
    pub security_both: bool,
    /// Assume referential integrity (DirectQuery hint; informational).
    pub assume_integrity: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Measure {
    pub table: String,
    pub name: String,
    pub expression: String,
    pub format: String,
    pub description: String,
    pub display_folder: String,
    pub hidden: bool,
    /// Created by "Quick measure" / a parameter / Tableau calculated field (informational).
    pub origin: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CalcColumn {
    pub table: String,
    pub name: String,
    pub expression: String,
    pub origin: String,
}

/// Parameters: Power BI what-if and field parameters, and Tableau parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Parameter {
    pub name: String,
    pub kind: ParameterKind,
    /// The current value (what-if / Tableau parameters; for field parameters, the selected item index).
    #[serde(default)]
    pub current: Scalar,
    #[serde(default)]
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ParameterKind {
    /// Numeric range: builds a table `GENERATESERIES(min, max, step)` and a `<name> Value` measure.
    WhatIf { min: f64, max: f64, step: f64, default: f64 },
    /// Switches the fields a visual uses: each item is a column or a measure.
    Fields { items: Vec<FieldRef> },
    /// A Tableau-style parameter: a typed value from a list, a range, or any value; referenced in
    /// DAX/calculated fields as `@Name` (replaced by its current value).
    Value {
        data_type: String,
        #[serde(default)]
        list: Vec<Scalar>,
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
        #[serde(default)]
        step: Option<f64>,
        /// Fill the list from a column's values when the project refreshes ("Table[Column]").
        #[serde(default)]
        from_field: Option<String>,
    },
}

/// Row-level security role: DAX filters per table.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Role {
    pub name: String,
    pub filters: Vec<(String, String)>,
    pub members: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiagramLayout {
    pub name: String,
    /// Table name → (x, y) in diagram points.
    pub positions: BTreeMap<String, (f32, f32)>,
    /// Tables shown in this layout (empty in "All tables" = every table).
    pub tables: Vec<String>,
    pub zoom: f32,
}

// ---------------------------------------------------------------------------------------------
// Fields
// ---------------------------------------------------------------------------------------------

/// A reference to something in the model a visual can use.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum FieldRef {
    Column { table: String, column: String },
    Measure { table: String, name: String },
    /// One level of a hierarchy (user hierarchy or the auto date hierarchy: levels
    /// `Year`, `Quarter`, `Month`, `Day` of a date column, hierarchy name = column name).
    Level { table: String, hierarchy: String, level: String },
    /// A field parameter: resolves to the item currently selected.
    FieldParam { name: String },
}

impl FieldRef {
    pub fn col(table: &str, column: &str) -> FieldRef {
        FieldRef::Column { table: table.into(), column: column.into() }
    }
    pub fn measure(table: &str, name: &str) -> FieldRef {
        FieldRef::Measure { table: table.into(), name: name.into() }
    }
    pub fn table(&self) -> &str {
        match self {
            FieldRef::Column { table, .. } | FieldRef::Measure { table, .. } | FieldRef::Level { table, .. } => table,
            FieldRef::FieldParam { .. } => "",
        }
    }
    /// What the user sees: `Region`, `Total Sales`, `Order Date Year`.
    pub fn label(&self) -> String {
        match self {
            FieldRef::Column { column, .. } => column.clone(),
            FieldRef::Measure { name, .. } => name.clone(),
            FieldRef::Level { hierarchy, level, .. } => format!("{hierarchy} {level}"),
            FieldRef::FieldParam { name } => name.clone(),
        }
    }
    pub fn is_measure(&self) -> bool {
        matches!(self, FieldRef::Measure { .. })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Agg {
    /// Group by the column (a category). Measures ignore it.
    #[default]
    None,
    Sum,
    Average,
    Min,
    Max,
    Count,
    DistinctCount,
    Median,
    StdDev,
    Variance,
    First,
    Last,
}

impl Agg {
    pub const ALL: [Agg; 12] =
        [Agg::None, Agg::Sum, Agg::Average, Agg::Min, Agg::Max, Agg::Count, Agg::DistinctCount, Agg::Median, Agg::StdDev, Agg::Variance, Agg::First, Agg::Last];
    pub fn label(self) -> &'static str {
        match self {
            Agg::None => "Don't summarize",
            Agg::Sum => "Sum",
            Agg::Average => "Average",
            Agg::Min => "Minimum",
            Agg::Max => "Maximum",
            Agg::Count => "Count",
            Agg::DistinctCount => "Count (Distinct)",
            Agg::Median => "Median",
            Agg::StdDev => "Standard deviation",
            Agg::Variance => "Variance",
            Agg::First => "First",
            Agg::Last => "Last",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            Agg::None => "",
            Agg::Sum => "Sum of",
            Agg::Average => "Average of",
            Agg::Min => "Min of",
            Agg::Max => "Max of",
            Agg::Count => "Count of",
            Agg::DistinctCount => "Count of",
            Agg::Median => "Median of",
            Agg::StdDev => "Std dev of",
            Agg::Variance => "Variance of",
            Agg::First => "First",
            Agg::Last => "Last",
        }
    }
    pub fn from_label(s: &str) -> Agg {
        Agg::ALL.into_iter().find(|a| a.label().eq_ignore_ascii_case(s) || format!("{a:?}").eq_ignore_ascii_case(s)).unwrap_or(Agg::None)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShowAs {
    #[default]
    Value,
    PercentOfGrandTotal,
    PercentOfColumnTotal,
    RunningTotal,
    DifferenceFromPrevious,
    PercentDifferenceFromPrevious,
    Rank,
}

/// A field placed in a well (axis, values, legend, …).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldUse {
    pub field: FieldRef,
    #[serde(default)]
    pub agg: Agg,
    /// Rename for this visual.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default)]
    pub show_as: ShowAs,
    /// Bin numeric categories into this width (Tableau bins / histogram).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<f64>,
    /// Show items with no data.
    #[serde(default)]
    pub show_empty: bool,
}

impl FieldUse {
    pub fn new(field: FieldRef) -> FieldUse {
        FieldUse { field, agg: Agg::None, alias: None, show_as: ShowAs::Value, bin: None, show_empty: false }
    }
    pub fn agg(field: FieldRef, agg: Agg) -> FieldUse {
        FieldUse { agg, ..FieldUse::new(field) }
    }
    /// True when this field yields a number per group (a measure, or an aggregated column).
    pub fn is_value(&self) -> bool {
        self.field.is_measure() || self.agg != Agg::None
    }
    pub fn label(&self) -> String {
        if let Some(a) = &self.alias {
            return a.clone();
        }
        match (&self.field, self.agg) {
            (FieldRef::Measure { .. }, _) | (_, Agg::None) => self.field.label(),
            (f, a) => format!("{} {}", a.short(), f.label()),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Report {
    pub pages: Vec<Page>,
    pub stories: Vec<Story>,
    pub bookmarks: Vec<Bookmark>,
    /// Filters on all pages.
    pub filters: Vec<FilterDef>,
    pub theme: Theme,
    pub settings: ReportSettings,
    /// Images embedded in the project (name → base64 PNG/JPEG/SVG).
    pub images: BTreeMap<String, String>,
    /// Shared slicer state: slicer sync groups (group name → selected values).
    pub sync_groups: BTreeMap<String, Vec<Scalar>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReportSettings {
    /// Cross-highlight (true) or cross-filter (false) by default when a data point is clicked.
    pub cross_highlight: bool,
    pub snap_to_grid: bool,
    pub grid_size: f32,
    pub show_gridlines: bool,
    pub lock_objects: bool,
    pub persistent_filters: bool,
    pub page_navigation_tabs: bool,
}

impl Default for ReportSettings {
    fn default() -> Self {
        ReportSettings {
            cross_highlight: true,
            snap_to_grid: true,
            grid_size: 8.0,
            show_gridlines: false,
            lock_objects: false,
            persistent_filters: false,
            page_navigation_tabs: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PageKind {
    /// A Power BI report page: a free canvas.
    #[default]
    Report,
    /// A Tableau worksheet: one visual that fills the page, built from shelves.
    Worksheet,
    /// A Tableau dashboard: worksheets and objects in tiled or floating layout, with actions.
    Dashboard,
    /// A tooltip page (shown when hovering over visuals that point to it).
    Tooltip,
    /// A drill-through target page.
    Drillthrough,
}

impl PageKind {
    pub fn label(self) -> &'static str {
        match self {
            PageKind::Report => "Report page",
            PageKind::Worksheet => "Worksheet",
            PageKind::Dashboard => "Dashboard",
            PageKind::Tooltip => "Tooltip page",
            PageKind::Drillthrough => "Drill-through page",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Page {
    pub id: u64,
    pub name: String,
    pub kind: PageKind,
    pub width: f32,
    pub height: f32,
    /// "16:9", "4:3", "Letter", "Tooltip", "Custom", "Phone".
    pub size_preset: String,
    pub background: Fill,
    pub wallpaper: Fill,
    pub background_image: Option<String>,
    pub elements: Vec<Element>,
    pub filters: Vec<FilterDef>,
    /// Drill-through fields (for drill-through pages).
    pub drillthrough: Vec<FieldRef>,
    pub keep_all_filters: bool,
    pub hidden: bool,
    pub mobile: MobileLayout,
    /// Dashboard actions (Tableau): filter, highlight, URL, go to sheet, change parameter.
    pub actions: Vec<DashboardAction>,
    /// For a worksheet: the element that is the sheet's visual.
    pub worksheet_visual: Option<u64>,
    /// For a worksheet opened from a visual on another page: that page.
    pub opened_from: Option<u64>,
    /// "Fit to page", "Fit to width", "Actual size".
    pub view_mode: String,
    pub vertical_align: String,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            id: 0,
            name: "Page".into(),
            kind: PageKind::Report,
            width: 1280.0,
            height: 720.0,
            size_preset: "16:9".into(),
            background: Fill::Solid(Color::WHITE),
            wallpaper: Fill::None,
            background_image: None,
            elements: vec![],
            filters: vec![],
            drillthrough: vec![],
            keep_all_filters: true,
            hidden: false,
            mobile: MobileLayout::default(),
            actions: vec![],
            worksheet_visual: None,
            opened_from: None,
            view_mode: "Fit to page".into(),
            vertical_align: "Top".into(),
        }
    }
}

impl Page {
    pub fn new(id: u64, name: &str, kind: PageKind) -> Page {
        let mut p = Page { id, name: name.into(), kind, ..Page::default() };
        if kind == PageKind::Tooltip {
            p.width = 320.0;
            p.height = 240.0;
            p.size_preset = "Tooltip".into();
        }
        p
    }
    pub fn element(&self, id: u64) -> Option<&Element> {
        self.elements.iter().find(|e| e.id == id)
    }
    pub fn element_mut(&mut self, id: u64) -> Option<&mut Element> {
        self.elements.iter_mut().find(|e| e.id == id)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MobileLayout {
    pub width: f32,
    /// Elements placed on the phone canvas, in order.
    pub items: Vec<MobileItem>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MobileItem {
    pub element: u64,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A Tableau-style story: a sequence of story points, each showing a page (sheet or dashboard)
/// with the filter state captured when the point was made, and a caption.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Story {
    pub id: u64,
    pub name: String,
    pub title: String,
    pub points: Vec<StoryPoint>,
    pub width: f32,
    pub height: f32,
    /// "Caption boxes", "Numbers", "Dots", "Arrows only".
    pub navigator_style: String,
    pub background: Fill,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StoryPoint {
    pub id: u64,
    pub caption: String,
    pub page: u64,
    pub state: ViewState,
    /// Annotation text boxes floating over the point (Tableau "description").
    pub notes: Vec<Element>,
}

// ---------------------------------------------------------------------------------------------
// Elements: visuals, shapes, text, images, buttons, groups
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Element {
    pub id: u64,
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Degrees, clockwise.
    pub rotation: f32,
    pub hidden: bool,
    pub locked: bool,
    /// The group this element belongs to.
    pub parent: Option<u64>,
    pub kind: ElementKind,
    pub style: BoxStyle,
    /// Per-element formatting properties (see [`crate::format`]): fixed values or fx bindings.
    pub props: BTreeMap<String, Prop>,
    /// How this element reacts when another element is clicked (source element id → interaction).
    pub interactions: BTreeMap<u64, Interaction>,
    /// Tooltip page to show on hover.
    pub tooltip_page: Option<u64>,
    /// Sync group for slicers.
    pub sync_group: Option<String>,
    /// Dashboard layout: tiled (in the layout flow) or floating.
    pub floating: bool,
}

impl Default for Element {
    fn default() -> Self {
        Element {
            id: 0,
            name: String::new(),
            x: 40.0,
            y: 40.0,
            w: 400.0,
            h: 260.0,
            rotation: 0.0,
            hidden: false,
            locked: false,
            parent: None,
            kind: ElementKind::Text(TextSpec::default()),
            style: BoxStyle::default(),
            props: BTreeMap::new(),
            interactions: BTreeMap::new(),
            tooltip_page: None,
            sync_group: None,
            floating: true,
        }
    }
}

impl Element {
    pub fn visual(&self) -> Option<&VisualSpec> {
        match &self.kind {
            ElementKind::Visual(v) => Some(v),
            _ => None,
        }
    }
    pub fn visual_mut(&mut self) -> Option<&mut VisualSpec> {
        match &mut self.kind {
            ElementKind::Visual(v) => Some(v),
            _ => None,
        }
    }
    pub fn kind_label(&self) -> String {
        match &self.kind {
            ElementKind::Visual(v) => v.kind.label().to_string(),
            ElementKind::Shape(s) => format!("{:?}", s.shape),
            ElementKind::Text(_) => "Text box".into(),
            ElementKind::Image(_) => "Image".into(),
            ElementKind::Button(_) => "Button".into(),
            ElementKind::Group => "Group".into(),
        }
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x <= self.x + self.w && y <= self.y + self.h
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ElementKind {
    Visual(VisualSpec),
    Shape(ShapeSpec),
    Text(TextSpec),
    Image(ImageSpec),
    Button(ButtonSpec),
    /// A Figma-style group: its children have `parent == Some(this id)`.
    Group,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "value")]
pub enum Fill {
    #[default]
    None,
    Solid(Color),
    /// Linear gradient: angle in degrees and colour stops (position 0..1, colour).
    Linear { angle: f32, stops: Vec<(f32, Color)> },
    /// Radial gradient from the centre.
    Radial { stops: Vec<(f32, Color)> },
}

impl Fill {
    /// The representative colour (for renderers without gradients).
    pub fn color(&self) -> Option<Color> {
        match self {
            Fill::None => None,
            Fill::Solid(c) => Some(*c),
            Fill::Linear { stops, .. } | Fill::Radial { stops } => stops.first().map(|s| s.1),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Stroke {
    pub width: f32,
    pub color: Color,
    /// "solid", "dash", "dot".
    pub dash: String,
}

impl Default for Stroke {
    fn default() -> Self {
        Stroke { width: 0.0, color: Color::rgb(0xCC, 0xCC, 0xCC), dash: "solid".into() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Shadow {
    pub enabled: bool,
    pub color: Color,
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
}

impl Default for Shadow {
    fn default() -> Self {
        Shadow { enabled: false, color: Color::rgba(0, 0, 0, 40), x: 0.0, y: 2.0, blur: 8.0, spread: 0.0 }
    }
}

/// The container look shared by every element (Figma-like).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BoxStyle {
    /// `None` = use the theme's visual background.
    pub fill: Option<Fill>,
    pub stroke: Stroke,
    /// Corner radius: one value or per-corner (tl, tr, br, bl).
    pub radius: [f32; 4],
    pub shadow: Shadow,
    pub opacity: f32,
    pub padding: f32,
    /// "normal", "multiply", "screen", "overlay" (web renderer).
    pub blend: String,
}

impl Default for BoxStyle {
    fn default() -> Self {
        BoxStyle { fill: None, stroke: Stroke::default(), radius: [-1.0; 4], shadow: Shadow::default(), opacity: 1.0, padding: 8.0, blend: "normal".into() }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShapeKind {
    #[default]
    Rectangle,
    RoundedRectangle,
    Ellipse,
    Line,
    Arrow,
    Triangle,
    Diamond,
    Pentagon,
    Hexagon,
    Star,
    Chevron,
    Speech,
    Heart,
    Plus,
    /// A free path (pen tool): points in 0..1 element-relative coordinates.
    Path,
    /// A frame: a Figma-style container with clip and auto-layout.
    Frame,
}

impl ShapeKind {
    pub const ALL: [ShapeKind; 16] = [
        ShapeKind::Rectangle,
        ShapeKind::RoundedRectangle,
        ShapeKind::Ellipse,
        ShapeKind::Line,
        ShapeKind::Arrow,
        ShapeKind::Triangle,
        ShapeKind::Diamond,
        ShapeKind::Pentagon,
        ShapeKind::Hexagon,
        ShapeKind::Star,
        ShapeKind::Chevron,
        ShapeKind::Speech,
        ShapeKind::Heart,
        ShapeKind::Plus,
        ShapeKind::Path,
        ShapeKind::Frame,
    ];
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShapeSpec {
    pub shape: ShapeKind,
    pub fill: Fill,
    pub stroke: Stroke,
    /// Path points (0..1 relative) for `Path`; closed when `closed`.
    pub points: Vec<(f32, f32)>,
    pub closed: bool,
    /// Text inside the shape (supports `{…}` templates).
    pub text: RichText,
    /// Frame auto layout: "none", "horizontal", "vertical".
    pub auto_layout: String,
    pub gap: f32,
    pub clip: bool,
}

impl Default for ShapeSpec {
    fn default() -> Self {
        ShapeSpec {
            shape: ShapeKind::Rectangle,
            fill: Fill::Solid(Color::rgb(0x2B, 0x6C, 0xE0)),
            stroke: Stroke::default(),
            points: vec![],
            closed: true,
            text: RichText::default(),
            auto_layout: "none".into(),
            gap: 8.0,
            clip: true,
        }
    }
}

/// Text made of runs. Each run's text may contain `{…}` placeholders (live values: measures,
/// DAX expressions, parameters, filter selections) and its colour may be an fx binding, so a
/// title can read "Sales are **{[Total Sales]:$#,0}** (▲ {[YoY %]:0.0%})" with the number
/// coloured by a rule.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RichText {
    pub runs: Vec<TextRun>,
    /// "left", "center", "right".
    pub align: String,
    /// "top", "middle", "bottom".
    pub valign: String,
    pub line_height: f32,
}

impl RichText {
    pub fn plain(s: &str) -> RichText {
        RichText { runs: vec![TextRun { text: s.into(), ..TextRun::default() }], ..RichText::default() }
    }
    pub fn raw_text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
    pub fn is_empty(&self) -> bool {
        self.runs.iter().all(|r| r.text.is_empty())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TextRun {
    pub text: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// `None` = theme foreground.
    pub color: Option<Prop>,
    pub font: String,
    pub link: Option<String>,
}

impl Default for TextRun {
    fn default() -> Self {
        TextRun { text: String::new(), size: 14.0, bold: false, italic: false, underline: false, color: None, font: String::new(), link: None }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TextSpec {
    pub text: RichText,
    /// Smart narrative: regenerate an automatic summary of the page's visuals.
    pub smart_narrative: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImageSpec {
    /// Key into `Report::images`, or a URL.
    pub source: String,
    /// "fit", "fill", "normal".
    pub scaling: String,
    pub action: Option<Action>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ButtonSpec {
    /// "Blank", "Back", "Bookmark", "Information", "Help", "Q&A", "Navigator", "Arrow", "Reset".
    pub icon: String,
    pub text: RichText,
    pub fill: Fill,
    pub hover_fill: Fill,
    pub action: Option<Action>,
    /// Page / bookmark navigator: generated buttons.
    pub navigator: Option<String>,
}

impl Default for ButtonSpec {
    fn default() -> Self {
        ButtonSpec {
            icon: "Blank".into(),
            text: RichText::plain("Button"),
            fill: Fill::Solid(Color::rgb(0x2B, 0x6C, 0xE0)),
            hover_fill: Fill::Solid(Color::rgb(0x1E, 0x55, 0xBA)),
            action: None,
            navigator: None,
        }
    }
}

/// What a button / image / shape does when clicked (Power BI button actions + Tableau
/// dashboard actions).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Action {
    Back,
    Bookmark { id: u64 },
    PageNavigation { page: u64 },
    Drillthrough { page: u64 },
    Url { url: String },
    QandA,
    ClearAllSlicers,
    SetParameter { name: String, value: Scalar },
    ApplyAllSlicers,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Interaction {
    #[default]
    Default,
    Filter,
    Highlight,
    None,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardAction {
    pub id: u64,
    pub name: String,
    /// "Filter", "Highlight", "URL", "Go to Sheet", "Change Parameter", "Change Set Value".
    pub kind: String,
    pub source: Vec<u64>,
    pub targets: Vec<u64>,
    /// "Select", "Hover", "Menu".
    pub run_on: String,
    /// When clearing the selection: "Keep filtered values", "Show all values", "Exclude all values".
    pub on_clear: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub target_page: Option<u64>,
    #[serde(default)]
    pub parameter: Option<String>,
    #[serde(default)]
    pub source_field: Option<FieldRef>,
}

// ---------------------------------------------------------------------------------------------
// Visuals
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VisualKind {
    StackedBar,
    #[default]
    StackedColumn,
    ClusteredBar,
    ClusteredColumn,
    StackedBar100,
    StackedColumn100,
    Line,
    Area,
    StackedArea,
    StackedArea100,
    LineStackedColumn,
    LineClusteredColumn,
    Ribbon,
    Waterfall,
    Funnel,
    Scatter,
    Pie,
    Donut,
    Treemap,
    Map,
    FilledMap,
    Gauge,
    Card,
    MultiRowCard,
    Kpi,
    Slicer,
    Table,
    Matrix,
    KeyInfluencers,
    DecompositionTree,
    QandA,
    SmartNarrative,
    PythonVisual,
    RVisual,
    // Tableau "Show Me" extras
    Histogram,
    BoxPlot,
    HeatMap,
    HighlightTable,
    Bullet,
    Gantt,
    PackedBubbles,
    WordCloud,
    Radar,
    Sankey,
    DualAxis,
    Sparkline,
}

impl VisualKind {
    pub const ALL: &'static [VisualKind] = &[
        VisualKind::StackedBar,
        VisualKind::StackedColumn,
        VisualKind::ClusteredBar,
        VisualKind::ClusteredColumn,
        VisualKind::StackedBar100,
        VisualKind::StackedColumn100,
        VisualKind::Line,
        VisualKind::Area,
        VisualKind::StackedArea,
        VisualKind::StackedArea100,
        VisualKind::LineStackedColumn,
        VisualKind::LineClusteredColumn,
        VisualKind::Ribbon,
        VisualKind::Waterfall,
        VisualKind::Funnel,
        VisualKind::Scatter,
        VisualKind::Pie,
        VisualKind::Donut,
        VisualKind::Treemap,
        VisualKind::Map,
        VisualKind::FilledMap,
        VisualKind::Gauge,
        VisualKind::Card,
        VisualKind::MultiRowCard,
        VisualKind::Kpi,
        VisualKind::Slicer,
        VisualKind::Table,
        VisualKind::Matrix,
        VisualKind::KeyInfluencers,
        VisualKind::DecompositionTree,
        VisualKind::QandA,
        VisualKind::SmartNarrative,
        VisualKind::PythonVisual,
        VisualKind::RVisual,
        VisualKind::Histogram,
        VisualKind::BoxPlot,
        VisualKind::HeatMap,
        VisualKind::HighlightTable,
        VisualKind::Bullet,
        VisualKind::Gantt,
        VisualKind::PackedBubbles,
        VisualKind::WordCloud,
        VisualKind::Radar,
        VisualKind::Sankey,
        VisualKind::DualAxis,
        VisualKind::Sparkline,
    ];

    pub fn label(self) -> &'static str {
        use VisualKind::*;
        match self {
            StackedBar => "Stacked bar chart",
            StackedColumn => "Stacked column chart",
            ClusteredBar => "Clustered bar chart",
            ClusteredColumn => "Clustered column chart",
            StackedBar100 => "100% stacked bar chart",
            StackedColumn100 => "100% stacked column chart",
            Line => "Line chart",
            Area => "Area chart",
            StackedArea => "Stacked area chart",
            StackedArea100 => "100% stacked area chart",
            LineStackedColumn => "Line and stacked column chart",
            LineClusteredColumn => "Line and clustered column chart",
            Ribbon => "Ribbon chart",
            Waterfall => "Waterfall chart",
            Funnel => "Funnel",
            Scatter => "Scatter chart",
            Pie => "Pie chart",
            Donut => "Donut chart",
            Treemap => "Treemap",
            Map => "Map",
            FilledMap => "Filled map",
            Gauge => "Gauge",
            Card => "Card",
            MultiRowCard => "Multi-row card",
            Kpi => "KPI",
            Slicer => "Slicer",
            Table => "Table",
            Matrix => "Matrix",
            KeyInfluencers => "Key influencers",
            DecompositionTree => "Decomposition tree",
            QandA => "Q&A",
            SmartNarrative => "Smart narrative",
            PythonVisual => "Python visual",
            RVisual => "R script visual",
            Histogram => "Histogram",
            BoxPlot => "Box-and-whisker plot",
            HeatMap => "Heat map",
            HighlightTable => "Highlight table",
            Bullet => "Bullet graph",
            Gantt => "Gantt chart",
            PackedBubbles => "Packed bubbles",
            WordCloud => "Word cloud",
            Radar => "Radar chart",
            Sankey => "Sankey diagram",
            DualAxis => "Dual-axis combo",
            Sparkline => "Sparkline",
        }
    }

    /// The field wells this visual offers, as (role id, label).
    pub fn wells(self) -> &'static [(&'static str, &'static str)] {
        use VisualKind::*;
        match self {
            StackedBar | ClusteredBar | StackedBar100 => &[("axis", "Y-axis"), ("values", "X-axis"), ("legend", "Legend"), ("small", "Small multiples"), ("tooltips", "Tooltips")],
            StackedColumn | ClusteredColumn | StackedColumn100 | Ribbon => {
                &[("axis", "X-axis"), ("values", "Y-axis"), ("legend", "Legend"), ("small", "Small multiples"), ("tooltips", "Tooltips")]
            }
            Line | Area | StackedArea | StackedArea100 => {
                &[("axis", "X-axis"), ("values", "Y-axis"), ("secondary", "Secondary y-axis"), ("legend", "Legend"), ("small", "Small multiples"), ("tooltips", "Tooltips")]
            }
            LineStackedColumn | LineClusteredColumn | DualAxis => {
                &[("axis", "X-axis"), ("values", "Column y-axis"), ("secondary", "Line y-axis"), ("legend", "Column legend"), ("tooltips", "Tooltips")]
            }
            Waterfall => &[("axis", "Category"), ("breakdown", "Breakdown"), ("values", "Y-axis"), ("tooltips", "Tooltips")],
            Funnel => &[("axis", "Category"), ("values", "Values"), ("tooltips", "Tooltips")],
            Scatter => &[("details", "Values"), ("x", "X-axis"), ("y", "Y-axis"), ("legend", "Legend"), ("size", "Size"), ("play", "Play axis"), ("tooltips", "Tooltips")],
            Pie | Donut => &[("legend", "Legend"), ("values", "Values"), ("details", "Details"), ("tooltips", "Tooltips")],
            Treemap => &[("axis", "Category"), ("details", "Details"), ("values", "Values"), ("tooltips", "Tooltips")],
            Map | FilledMap => &[("location", "Location"), ("lat", "Latitude"), ("lon", "Longitude"), ("legend", "Legend"), ("size", "Bubble size"), ("tooltips", "Tooltips")],
            Gauge => &[("values", "Value"), ("min", "Minimum value"), ("max", "Maximum value"), ("target", "Target value"), ("tooltips", "Tooltips")],
            Card => &[("values", "Fields")],
            MultiRowCard => &[("values", "Fields")],
            Kpi => &[("values", "Value"), ("axis", "Trend axis"), ("target", "Target")],
            Slicer => &[("field", "Field")],
            Table => &[("columns", "Columns")],
            Matrix => &[("rows", "Rows"), ("columns", "Columns"), ("values", "Values")],
            KeyInfluencers => &[("values", "Analyze"), ("details", "Explain by"), ("expand", "Expand by")],
            DecompositionTree => &[("values", "Analyze"), ("details", "Explain by")],
            QandA | SmartNarrative => &[("values", "Fields")],
            PythonVisual | RVisual => &[("values", "Values")],
            Histogram => &[("axis", "Value (binned)"), ("values", "Count of"), ("legend", "Legend")],
            BoxPlot => &[("axis", "Category"), ("details", "Sampling"), ("values", "Value")],
            HeatMap | HighlightTable => &[("rows", "Rows"), ("columns", "Columns"), ("values", "Color")],
            Bullet => &[("axis", "Category"), ("values", "Value"), ("target", "Target"), ("min", "Range")],
            Gantt => &[("axis", "Task"), ("start", "Start"), ("values", "Duration"), ("legend", "Legend")],
            PackedBubbles | WordCloud => &[("axis", "Category"), ("values", "Size"), ("legend", "Color")],
            Radar => &[("axis", "Category"), ("values", "Values"), ("legend", "Legend")],
            Sankey => &[("source", "Source"), ("destination", "Destination"), ("values", "Weight")],
            Sparkline => &[("axis", "Axis"), ("values", "Value")],
        }
    }

    pub fn group(self) -> &'static str {
        use VisualKind::*;
        match self {
            StackedBar | StackedColumn | ClusteredBar | ClusteredColumn | StackedBar100 | StackedColumn100 | Ribbon | Waterfall | Funnel | Histogram | Bullet => "Bar & column",
            Line | Area | StackedArea | StackedArea100 | LineStackedColumn | LineClusteredColumn | DualAxis | Sparkline => "Line & area",
            Scatter | BoxPlot | PackedBubbles | Radar => "Distribution",
            Pie | Donut | Treemap | Sankey | WordCloud => "Part to whole",
            Map | FilledMap => "Maps",
            Gauge | Card | MultiRowCard | Kpi => "Cards & KPIs",
            Slicer | Table | Matrix | HeatMap | HighlightTable | Gantt => "Tables & slicers",
            KeyInfluencers | DecompositionTree | QandA | SmartNarrative => "AI visuals",
            PythonVisual | RVisual => "Script visuals",
        }
    }

    pub fn is_cartesian(self) -> bool {
        use VisualKind::*;
        matches!(
            self,
            StackedBar
                | StackedColumn
                | ClusteredBar
                | ClusteredColumn
                | StackedBar100
                | StackedColumn100
                | Line
                | Area
                | StackedArea
                | StackedArea100
                | LineStackedColumn
                | LineClusteredColumn
                | Ribbon
                | Waterfall
                | Scatter
                | Histogram
                | DualAxis
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VisualSpec {
    pub kind: VisualKind,
    /// Role id → fields (see [`VisualKind::wells`]).
    pub wells: BTreeMap<String, Vec<FieldUse>>,
    /// Visual-level filters.
    pub filters: Vec<FilterDef>,
    pub analytics: Vec<AnalyticsLine>,
    pub sort: Option<SortSpec>,
    /// Drill state for hierarchies on the axis/rows.
    pub drill: DrillState,
    /// Python / R visuals.
    pub script: String,
    /// Slicer mode: "List", "Dropdown", "Tile", "Between", "Before", "After", "Relative date".
    pub slicer_mode: String,
    /// Slicer / Q&A input state.
    pub selection: Vec<Scalar>,
    pub range: Option<(f64, f64)>,
    pub question: String,
    /// Matrix: rows expanded (keys of expanded row paths).
    pub expanded: Vec<String>,
    /// Tableau marks card (worksheets): mark type override and encodings.
    pub marks: MarksCard,
}

impl VisualSpec {
    pub fn new(kind: VisualKind) -> VisualSpec {
        VisualSpec { kind, slicer_mode: "List".into(), ..VisualSpec::default() }
    }
    pub fn well(&self, role: &str) -> &[FieldUse] {
        self.wells.get(role).map(|v| v.as_slice()).unwrap_or(&[])
    }
    pub fn well_mut(&mut self, role: &str) -> &mut Vec<FieldUse> {
        self.wells.entry(role.to_string()).or_default()
    }
    pub fn set(&mut self, role: &str, fields: Vec<FieldUse>) {
        self.wells.insert(role.to_string(), fields);
    }
    pub fn all_fields(&self) -> impl Iterator<Item = &FieldUse> {
        self.wells.values().flatten()
    }
}

/// Tableau marks card: how marks are drawn and which fields encode colour, size, label, detail,
/// tooltip and shape. Fields placed here are added to the visual's group-by.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MarksCard {
    /// "Automatic", "Bar", "Line", "Area", "Square", "Circle", "Shape", "Text", "Pie", "Gantt Bar", "Polygon", "Density".
    pub mark: String,
    pub color: Vec<FieldUse>,
    pub size: Vec<FieldUse>,
    pub label: Vec<FieldUse>,
    pub detail: Vec<FieldUse>,
    pub tooltip: Vec<FieldUse>,
    pub shape: Vec<FieldUse>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SortSpec {
    /// The label of the field to sort by (category or value).
    pub by: String,
    pub descending: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DrillState {
    /// Drill mode on: clicking a data point drills down instead of cross-filtering.
    pub drill_mode: bool,
    /// The current level index into the axis/rows hierarchy.
    pub level: usize,
    /// Values of the parent levels drilled into (`[2024, "Q1"]`).
    pub path: Vec<Scalar>,
    /// "Expand all down one level" (show all levels concatenated) up to this level.
    pub expand_to: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AnalyticsLine {
    /// "Constant", "Min", "Max", "Average", "Median", "Percentile", "Trend", "Forecast", "Error bars",
    /// "Cluster", "Anomalies", "Reference band", "Distribution".
    pub kind: String,
    pub name: String,
    /// Which value series (label) the line applies to; empty = first.
    pub series: String,
    pub value: Prop,
    pub percentile: f64,
    pub color: Color,
    pub width: f32,
    pub dash: String,
    pub show_label: bool,
    // Forecast
    pub forecast_length: usize,
    pub seasonality: usize,
    pub confidence: f64,
    pub ignore_last: usize,
    // Clustering
    pub clusters: usize,
    pub band_to: Option<Prop>,
}

impl Default for AnalyticsLine {
    fn default() -> Self {
        AnalyticsLine {
            kind: "Average".into(),
            name: String::new(),
            series: String::new(),
            value: Prop::Num(0.0),
            percentile: 50.0,
            color: Color::rgb(0x55, 0x55, 0x55),
            width: 1.5,
            dash: "dash".into(),
            show_label: true,
            forecast_length: 6,
            seasonality: 0,
            confidence: 0.95,
            ignore_last: 0,
            clusters: 3,
            band_to: None,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Properties (fx)
// ---------------------------------------------------------------------------------------------

/// A formatting property value. Every property can be fixed or bound to data (fx), which is
/// how titles show live numbers, colours follow rules, and any setting changes dynamically.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "t", content = "v")]
pub enum Prop {
    Bool(bool),
    Num(f64),
    Text(String),
    Color(Color),
    /// A choice from a list (font, alignment, position, …).
    Choice(String),
    Fx(Fx),
}

impl Default for Prop {
    fn default() -> Self {
        Prop::Text(String::new())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum Fx {
    /// Text with `{…}` placeholders: `{[Total Sales]}`, `{[Total Sales]:$#,0}`,
    /// `{SUM(Sales[Qty])}`, `{@Param}`, `{Selected:Region}`.
    Template { text: String },
    /// A DAX expression; its result is the property value (a number, text or a colour name/hex).
    Expression { dax: String },
    /// Rules: the first rule whose condition holds on `dax`'s value gives the output.
    Rules { dax: String, rules: Vec<FxRule>, default: Box<Prop> },
    /// Colour scale on `dax`'s value between min and max (auto when None).
    Gradient {
        dax: String,
        min: Option<f64>,
        max: Option<f64>,
        min_color: Color,
        #[serde(default)]
        mid_color: Option<Color>,
        max_color: Color,
    },
    /// A field whose value is the property (colour hex, URL, text).
    FieldValue { dax: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FxRule {
    /// ">", ">=", "<", "<=", "=", "<>", "between", "contains", "is blank".
    pub op: String,
    pub value: f64,
    #[serde(default)]
    pub value2: f64,
    #[serde(default)]
    pub text: String,
    /// Values are percentages of the range rather than numbers.
    #[serde(default)]
    pub percent: bool,
    pub output: Prop,
}

// ---------------------------------------------------------------------------------------------
// Filters, bookmarks, view state
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterDef {
    pub id: u64,
    pub field: FieldUse,
    pub kind: FilterKind,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub require_single: bool,
    /// Tableau-like context filter (applied before Top N).
    #[serde(default)]
    pub context: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum FilterKind {
    /// Basic filtering: keep (or exclude) the listed values. Empty + !exclude = no filter.
    Basic { values: Vec<Scalar>, exclude: bool },
    /// Advanced: one or two conditions joined by and/or.
    Advanced { conditions: Vec<Condition>, and: bool },
    /// Top / bottom N of the field by a value.
    TopN { n: usize, top: bool, by: Option<FieldUse> },
    /// Relative date: last/next/this N days/weeks/months/years.
    RelativeDate { mode: String, n: i64, unit: String, include_today: bool },
    /// Numeric/date range (between).
    Range { min: Option<f64>, max: Option<f64> },
    /// A DAX boolean filter expression.
    Dax { expression: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    /// "is", "is not", "is less than", "is less than or equal to", "is greater than",
    /// "is greater than or equal to", "contains", "does not contain", "starts with",
    /// "does not start with", "is blank", "is not blank".
    pub op: String,
    pub value: Scalar,
}

/// Everything interactive on the report that bookmarks and story points capture.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ViewState {
    /// Page-, visual- and report-level filters (by filter id → filter).
    pub filters: BTreeMap<u64, FilterKind>,
    /// Slicer selections (element id → values / range).
    pub slicers: BTreeMap<u64, (Vec<Scalar>, Option<(f64, f64)>)>,
    /// Cross-filter selection: source element → selected points.
    pub selection: Option<Selection>,
    /// Drill state per visual.
    pub drill: BTreeMap<u64, DrillState>,
    /// Element visibility.
    pub hidden: BTreeMap<u64, bool>,
    /// Parameter values.
    pub parameters: BTreeMap<String, Scalar>,
    /// Spotlight / focus mode element.
    pub focus: Option<u64>,
}

/// Points selected in a visual (cross-filter / cross-highlight source).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Selection {
    pub source: u64,
    /// Each selected point is a set of field = value conditions (category and series).
    pub points: Vec<Vec<(FieldRef, Scalar)>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Bookmark {
    pub id: u64,
    pub name: String,
    pub group: String,
    pub page: u64,
    pub state: ViewState,
    pub capture_data: bool,
    pub capture_display: bool,
    pub capture_current_page: bool,
    pub all_visuals: bool,
    /// When not all visuals: which elements the bookmark applies to.
    pub selected_visuals: Vec<u64>,
}

// ---------------------------------------------------------------------------------------------
// Flows (Fabric / KNIME / Orange style pipelines)
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Flow {
    pub name: String,
    pub nodes: Vec<FlowNode>,
    /// (from node, from port, to node, to port).
    pub links: Vec<(u64, usize, u64, usize)>,
    pub description: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FlowNode {
    pub id: u64,
    /// The node type id, e.g. "source.query", "transform.filter", "ml.kmeans" (see [`crate::flow`]).
    pub kind: String,
    pub label: String,
    pub x: f32,
    pub y: f32,
    /// Node settings.
    pub config: BTreeMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------------------------
// Publishing
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PublishSettings {
    pub targets: Vec<PublishTarget>,
    /// Include interactive filtering (ships the WASM engine with the data).
    pub interactive: bool,
    pub include_mobile: bool,
    pub last_published: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum PublishTarget {
    /// A folder on disk (static web site).
    Folder { path: String },
    /// One self-contained HTML file (share like an artifact).
    SingleFile { path: String },
    /// A VizCraft Service (the `vizcraft-server` portal): URL, workspace and token.
    Service { url: String, workspace: String, token: String },
    /// GitHub Pages: repository (`owner/repo`), branch, token, custom domain (CNAME).
    GitHubPages { repo: String, branch: String, token: String, domain: String, path: String },
}
